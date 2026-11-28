-- Local OCR for CLEAN immutable originals; candidate results only.
ALTER TABLE finance.ocr_run DROP CONSTRAINT ocr_run_engine_check,
 ADD CONSTRAINT ocr_run_engine_check CHECK(engine IN ('TESSERACT_SYNTHETIC','TESSERACT_LOCAL')),
 ADD COLUMN classification jsonb,
 ADD CONSTRAINT ocr_classification_bound CHECK(classification IS NULL OR (jsonb_typeof(classification)='array' AND jsonb_array_length(classification)=page_count));
ALTER TABLE finance.extraction_run DROP CONSTRAINT extraction_run_schema_version_check,
 ADD CONSTRAINT extraction_run_schema_version_check CHECK(schema_version IN ('synthetic.rows.v1','local.rows.v2'));
ALTER TABLE finance.extracted_field DROP CONSTRAINT extracted_field_field_path_check,
 ADD CONSTRAINT extracted_field_field_path_check CHECK(field_path IN (
 'invoice.number','invoice.date','invoice.supplier','invoice.net','invoice.tax','invoice.total',
 'contract.number','contract.date','contract.supplier','contract.total',
 'contract_appendix.number','contract_appendix.date','contract_appendix.supplier','contract_appendix.total',
 'payment_request.contract_number','payment_request.invoice_number','payment_request.date','payment_request.supplier','payment_request.requested',
 'advance_request.contract_number','advance_request.date','advance_request.requested',
 'advance_settlement.contract_number','advance_settlement.date','advance_settlement.requested',
 'acceptance_record.contract_number','acceptance_record.date','acceptance_record.total',
 'payment_voucher.date','payment_voucher.requested'));

CREATE FUNCTION finance.local_ocr_source(t uuid,c uuid,j uuid,f bigint,w uuid)
RETURNS TABLE(document_id uuid,revision_id uuid,generation bigint,source_digest bytea,content bytea,object_version text,byte_count bigint,mime text,kind text,event_type text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT b.id,r.revision_id,b.generation,b.sha256_digest,b.synthetic_bytes,b.object_version,b.byte_count,b.mime,u.document_kind,o.event_type
 FROM finance.job q
 JOIN finance.outbox o ON (o.tenant_id,o.company_id,o.id)=(q.tenant_id,q.company_id,q.id)
 JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(o.tenant_id,o.company_id,o.resource_id)
 JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(b.tenant_id,b.company_id,b.id)
 JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id,u.revision_id)=(r.tenant_id,r.company_id,r.id,r.revision_id)
 JOIN finance.dossier d ON (d.tenant_id,d.company_id,d.id)=(r.tenant_id,r.company_id,r.dossier_id)
 JOIN finance.dossier_revision rev ON (rev.tenant_id,rev.company_id,rev.dossier_id,rev.id,rev.number)
   =(d.tenant_id,d.company_id,d.id,r.revision_id,d.revision)
 WHERE t=finance.scope_tenant() AND c=finance.scope_company() AND finance.worker_scope_allowed(t,c)
 AND (q.tenant_id,q.company_id,q.id,q.fence,q.leased_by)=(t,c,j,f,w)
 AND q.state='LEASED' AND q.lease_until>statement_timestamp() AND o.event_type IN ('OCR_REQUESTED','CLASSIFY_REQUESTED','EXTRACT_REQUESTED')
 AND b.scan_state='CLEAN' AND b.storage_backend IN ('SYNTHETIC_DB_ONLY','S3_VERSIONED') AND b.mime IN ('image/png','application/pdf')
 AND u.state='FINALIZED' AND u.generation=b.generation AND u.document_kind IN ('INVOICE','CONTRACT','CONTRACT_APPENDIX','PAYMENT_REQUEST','ADVANCE_REQUEST','ADVANCE_SETTLEMENT','ACCEPTANCE_RECORD','PAYMENT_VOUCHER')
 AND d.status IN ('DRAFT','PROCESSING','NEEDS_INPUT','REVIEWABLE')
$$;
ALTER FUNCTION finance.local_ocr_source(uuid,uuid,uuid,bigint,uuid) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.local_ocr_source(uuid,uuid,uuid,bigint,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.local_ocr_source(uuid,uuid,uuid,bigint,uuid) TO finance_job_worker;



CREATE FUNCTION finance.complete_local_ocr(t uuid,c uuid,j uuid,f bigint,w uuid,
 doc uuid,rev uuid,g bigint,source bytea,model bytea,config bytea,blocks jsonb,fields jsonb,classification jsonb,pages integer,kind text,evt text)
RETURNS uuid LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE jobrow finance.job%ROWTYPE; b finance.document_blob%ROWTYPE; p jsonb; entry jsonb; idx integer; block_page integer;
 rect integer[]; ref_ids integer[]; page_no integer; field_count integer:=0; run_id uuid:=j;
 payload_digest bytea; dossier uuid; expected_paths text[];
BEGIN
 IF t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR NOT finance.worker_scope_allowed(t,c)
 THEN RAISE EXCEPTION 'worker authority' USING ERRCODE='42501'; END IF;
 IF current_setting('transaction_isolation')<>'read committed'
 OR t IS NULL OR c IS NULL OR j IS NULL OR w IS NULL OR doc IS NULL OR rev IS NULL OR f IS NULL OR g IS NULL
 OR source IS NULL OR octet_length(source)<>32 OR model IS NULL OR octet_length(model)<>32
 OR config IS NULL OR octet_length(config)<>32
 OR pages IS NULL OR pages NOT BETWEEN 1 AND 200 OR kind IS NULL OR evt IS NULL OR evt NOT IN ('OCR_REQUESTED','CLASSIFY_REQUESTED','EXTRACT_REQUESTED')
 OR jsonb_typeof(classification) IS DISTINCT FROM 'array' OR jsonb_array_length(classification)<>pages OR octet_length(classification::text)>1048576
 OR jsonb_typeof(blocks) IS DISTINCT FROM 'array' OR jsonb_array_length(blocks)>100000
 OR octet_length(blocks::text)>33554432 OR jsonb_typeof(fields) IS DISTINCT FROM 'array'
 OR jsonb_array_length(fields) NOT BETWEEN 1 AND 16 OR octet_length(fields::text)>65536
 THEN RAISE EXCEPTION 'invalid synthetic OCR payload' USING ERRCODE='23514'; END IF;
 SELECT * INTO jobrow FROM finance.job WHERE tenant_id=t AND company_id=c AND id=j FOR UPDATE;
 IF NOT FOUND OR jobrow.state<>'LEASED' OR jobrow.fence<>f OR jobrow.leased_by<>w OR jobrow.lease_until<=clock_timestamp()
 THEN RAISE EXCEPTION 'stale OCR lease' USING ERRCODE='40001'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.outbox WHERE tenant_id=t AND company_id=c AND id=j AND resource_id=doc AND event_type=evt)
 THEN RAISE EXCEPTION 'wrong OCR job source' USING ERRCODE='23514'; END IF;
 -- Lock the owner aggregate before checking the current revision and writing.
 SELECT r.dossier_id INTO dossier FROM finance.document_ref r
  WHERE (r.tenant_id,r.company_id,r.id,r.revision_id)=(t,c,doc,rev);
 IF NOT FOUND THEN RAISE EXCEPTION 'wrong OCR revision' USING ERRCODE='23514'; END IF;
 PERFORM 1 FROM finance.dossier d
  WHERE (d.tenant_id,d.company_id,d.id)=(t,c,dossier) FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'wrong OCR dossier' USING ERRCODE='23514'; END IF;
 SELECT * INTO b FROM finance.document_blob WHERE tenant_id=t AND company_id=c AND id=doc;
 IF NOT FOUND OR b.generation<>g OR b.sha256_digest<>source OR b.storage_backend NOT IN ('SYNTHETIC_DB_ONLY','S3_VERSIONED')
 OR b.scan_state<>'CLEAN' OR b.mime NOT IN ('image/png','application/pdf')
 OR NOT EXISTS(SELECT 1 FROM finance.local_ocr_source(t,c,j,f,w) s
   WHERE s.document_id=doc AND s.revision_id=rev AND s.generation=g AND s.source_digest=source AND s.kind=kind AND s.event_type=evt)
 THEN RAISE EXCEPTION 'wrong OCR source' USING ERRCODE='23514'; END IF;
 expected_paths:=CASE kind
 WHEN 'INVOICE' THEN ARRAY['invoice.number','invoice.date','invoice.supplier','invoice.net','invoice.tax','invoice.total']
 WHEN 'CONTRACT' THEN ARRAY['contract.number','contract.date','contract.supplier','contract.total']
 WHEN 'CONTRACT_APPENDIX' THEN ARRAY['contract_appendix.number','contract_appendix.date','contract_appendix.supplier','contract_appendix.total']
 WHEN 'PAYMENT_REQUEST' THEN ARRAY['payment_request.contract_number','payment_request.invoice_number','payment_request.date','payment_request.supplier','payment_request.requested']
 WHEN 'ADVANCE_REQUEST' THEN ARRAY['advance_request.contract_number','advance_request.date','advance_request.requested']
 WHEN 'ADVANCE_SETTLEMENT' THEN ARRAY['advance_settlement.contract_number','advance_settlement.date','advance_settlement.requested']
 WHEN 'ACCEPTANCE_RECORD' THEN ARRAY['acceptance_record.contract_number','acceptance_record.date','acceptance_record.total']
 WHEN 'PAYMENT_VOUCHER' THEN ARRAY['payment_voucher.date','payment_voucher.requested']
 ELSE NULL END;
 IF expected_paths IS NULL OR jsonb_array_length(fields)<>cardinality(expected_paths)
 THEN RAISE EXCEPTION 'unsupported extraction schema' USING ERRCODE='23514'; END IF;
 payload_digest:=sha256(convert_to(jsonb_build_array(blocks,fields,classification,pages)::text,'UTF8'));
 INSERT INTO finance.ocr_run(tenant_id,company_id,id,document_id,revision_id,source_digest,generation,job_id,fence,engine,model_digest,config_digest,page_count,result_digest,classification)
 VALUES(t,c,run_id,doc,rev,source,g,j,f,'TESSERACT_LOCAL',model,config,pages,payload_digest,classification);
 FOR p IN SELECT value FROM jsonb_array_elements(blocks) LOOP
  IF jsonb_typeof(p) IS DISTINCT FROM 'object' OR jsonb_typeof(p->'rect') IS DISTINCT FROM 'array'
  OR jsonb_array_length(p->'rect')<>4 OR jsonb_typeof(p->'text') IS DISTINCT FROM 'string'
  THEN RAISE EXCEPTION 'invalid OCR block' USING ERRCODE='23514'; END IF;
  SELECT array_agg(x::integer ORDER BY ord) INTO rect FROM jsonb_array_elements_text(p->'rect') WITH ORDINALITY AS a(x,ord);
  block_page:=(p->>'page')::integer; idx:=(p->>'index')::integer;
  IF block_page IS NULL OR block_page NOT BETWEEN 1 AND pages OR idx IS NULL OR idx NOT BETWEEN 0 AND 9999 THEN RAISE EXCEPTION 'invalid OCR page/index' USING ERRCODE='23514'; END IF;
  INSERT INTO finance.evidence(tenant_id,company_id,id,ocr_run_id,page,block_index,x0,y0,x1,y1,excerpt,confidence)
  VALUES(t,c,gen_random_uuid(),run_id,block_page,idx,rect[1],rect[2],rect[3],rect[4],p->>'text',(p->>'confidence')::integer);

 END LOOP;
 INSERT INTO finance.extraction_run VALUES(t,c,run_id,run_id,'local.rows.v2');
 FOR entry IN SELECT value FROM jsonb_array_elements(fields) LOOP
  IF jsonb_typeof(entry) IS DISTINCT FROM 'object' OR jsonb_typeof(entry->'indexes') IS DISTINCT FROM 'array'
  OR jsonb_array_length(entry->'indexes')>16 OR entry->>'path' IS NULL OR NOT (entry->>'path'=ANY(expected_paths)) THEN RAISE EXCEPTION 'invalid extracted field' USING ERRCODE='23514'; END IF;
  SELECT COALESCE(array_agg(x::integer ORDER BY ord),ARRAY[]::integer[]) INTO ref_ids FROM jsonb_array_elements_text(entry->'indexes') WITH ORDINALITY AS a(x,ord);
  page_no:=(entry->>'page')::integer;
  IF (entry->>'state'='CANDIDATE' AND cardinality(ref_ids)=0)
   OR cardinality(ref_ids)<>(SELECT count(DISTINCT k) FROM unnest(ref_ids) k)
   OR EXISTS(SELECT 1 FROM unnest(ref_ids) k WHERE NOT EXISTS(SELECT 1 FROM finance.evidence e
      WHERE e.tenant_id=t AND e.company_id=c AND e.ocr_run_id=run_id AND e.page=page_no AND e.block_index=k))
  THEN RAISE EXCEPTION 'unbound extraction evidence' USING ERRCODE='23514'; END IF;
  INSERT INTO finance.extracted_field VALUES(t,c,run_id,entry->>'path',entry->>'state',entry->>'value',page_no,ref_ids);
  field_count:=field_count+1;
 END LOOP;
 IF field_count<>cardinality(expected_paths) OR jobrow.lease_until<=clock_timestamp() THEN RAISE EXCEPTION 'incomplete result or expired lease' USING ERRCODE='23514'; END IF;
 UPDATE finance.job_attempt SET state='SUCCEEDED',finished_at=clock_timestamp() WHERE tenant_id=t AND company_id=c AND job_id=j AND fence=f AND worker_id=w AND state='ACTIVE';
 IF NOT FOUND THEN RAISE EXCEPTION 'stale OCR attempt' USING ERRCODE='23514'; END IF;
 UPDATE finance.job SET state='SUCCEEDED',leased_by=NULL,lease_until=NULL WHERE tenant_id=t AND company_id=c AND id=j
  AND fence=f AND leased_by=w AND state='LEASED' AND lease_until>clock_timestamp();
 IF NOT FOUND THEN RAISE EXCEPTION 'stale OCR lease' USING ERRCODE='40001'; END IF;
 INSERT INTO finance.inbox(tenant_id,company_id,event_id,fence) VALUES(t,c,j,f);
 RETURN run_id;
END $$;
REVOKE ALL ON FUNCTION finance.complete_local_ocr(uuid,uuid,uuid,bigint,uuid,uuid,uuid,bigint,bytea,bytea,bytea,jsonb,jsonb,jsonb,integer,text,text) FROM PUBLIC;

ALTER FUNCTION finance.complete_local_ocr(uuid,uuid,uuid,bigint,uuid,uuid,uuid,bigint,bytea,bytea,bytea,jsonb,jsonb,jsonb,integer,text,text) OWNER TO finance_workflow_writer;
GRANT EXECUTE ON FUNCTION finance.complete_local_ocr(uuid,uuid,uuid,bigint,uuid,uuid,uuid,bigint,bytea,bytea,bytea,jsonb,jsonb,jsonb,integer,text,text) TO finance_job_worker;


CREATE OR REPLACE FUNCTION finance.field_access_allowed(t uuid,c uuid,d uuid,a uuid)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT t=finance.scope_tenant() AND c=finance.scope_company() AND a=finance.scope_actor()
 AND EXISTS(
  SELECT 1 FROM finance.dossier ds
  JOIN finance.dossier_revision dr
    ON (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number)=(ds.tenant_id,ds.company_id,ds.id,ds.revision)
  WHERE ds.tenant_id=t AND ds.company_id=c AND ds.id=d
  AND (
   (ds.owner_id=a AND EXISTS(
    SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c AND m.actor_id=a
     AND m.role='REQUESTER' AND m.active AND m.valid_from<=statement_timestamp()
     AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())
   ))
   OR
   (EXISTS(
    SELECT 1 FROM finance.review_assignment ra
    WHERE ra.tenant_id=t AND ra.company_id=c AND ra.dossier_id=d AND ra.revision_id=dr.id AND ra.accountant_id=a
   ) AND EXISTS(
    SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c AND m.actor_id=a
     AND m.role='ACCOUNTANT' AND m.active AND m.valid_from<=statement_timestamp()
     AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())
   ))
  )
 )
$$;
ALTER FUNCTION finance.field_access_allowed(uuid,uuid,uuid,uuid) OWNER TO finance_field_review_writer;
REVOKE ALL ON FUNCTION finance.field_access_allowed(uuid,uuid,uuid,uuid) FROM PUBLIC;

CREATE FUNCTION finance.runtime_fields(t uuid,c uuid,d uuid,a uuid,after_id uuid,only_id uuid,n integer)
RETURNS TABLE(
 id uuid,path text,raw_value text,normalized_value text,value_type text,state text,
 evidence_ids uuid[],extractor_version text
)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF n IS NULL OR n NOT BETWEEN 1 AND 100 OR after_id IS NULL
 OR NOT finance.field_access_allowed(t,c,d,a)
 THEN RAISE EXCEPTION 'field read forbidden or invalid' USING ERRCODE='42501'; END IF;
 RETURN QUERY
 WITH current_rows AS (
  SELECT fi.id,f.run_id,f.field_path,f.state AS source_state,f.normalized_value AS source_value,
   f.source_page,f.block_indexes,x.schema_version,o.id AS ocr_id,
   ds.version AS dossier_version,fd.disposition,fd.normalized_value AS decision_value,
   fd.evidence_ids AS decision_evidence
  FROM finance.dossier ds
  JOIN finance.dossier_revision dr
    ON (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number)=(ds.tenant_id,ds.company_id,ds.id,ds.revision)
  JOIN finance.document_ref r
    ON (r.tenant_id,r.company_id,r.dossier_id,r.revision_id)=(ds.tenant_id,ds.company_id,ds.id,dr.id)
  JOIN finance.ocr_run o
    ON (o.tenant_id,o.company_id,o.document_id,o.revision_id)=(r.tenant_id,r.company_id,r.id,dr.id)
  JOIN finance.extraction_run x
    ON (x.tenant_id,x.company_id,x.ocr_run_id)=(o.tenant_id,o.company_id,o.id)
  JOIN finance.extracted_field f
    ON (f.tenant_id,f.company_id,f.run_id)=(x.tenant_id,x.company_id,x.id)
  JOIN finance.field_identity fi
    ON (fi.tenant_id,fi.company_id,fi.run_id,fi.field_path)=(f.tenant_id,f.company_id,f.run_id,f.field_path)
  LEFT JOIN LATERAL (
   SELECT q.disposition,q.normalized_value,q.evidence_ids
   FROM finance.field_decision q
   WHERE q.tenant_id=f.tenant_id AND q.company_id=f.company_id AND q.run_id=f.run_id
    AND q.field_path=f.field_path AND q.dossier_id=ds.id AND q.revision_id=dr.id
   ORDER BY q.dossier_version DESC,q.created_at DESC,q.id DESC LIMIT 1
  ) fd ON true
  WHERE ds.tenant_id=t AND ds.company_id=c AND ds.id=d
   AND (only_id IS NULL OR fi.id=only_id)
   AND (only_id IS NOT NULL OR fi.id>after_id)
   AND NOT EXISTS(
    SELECT 1 FROM finance.ocr_run newer
    WHERE newer.tenant_id=o.tenant_id AND newer.company_id=o.company_id
     AND newer.document_id=o.document_id AND newer.revision_id=o.revision_id
     AND (newer.created_at>o.created_at OR (newer.created_at=o.created_at AND newer.id>o.id))
   )
  ORDER BY fi.id
  LIMIT n
 )
 SELECT row.id,row.field_path,
  (SELECT string_agg(e.excerpt,' | ' ORDER BY e.block_index,e.id)
   FROM finance.evidence e
   WHERE e.tenant_id=t AND e.company_id=c AND e.ocr_run_id=row.ocr_id
    AND e.page=row.source_page AND e.block_index=ANY(row.block_indexes)) AS raw_value,
  CASE
   WHEN row.disposition IN ('CONFIRMED_CANDIDATE','CORRECTED') THEN row.decision_value
   WHEN row.disposition='REJECTED' THEN NULL
   WHEN row.source_state='CANDIDATE' THEN row.source_value
   ELSE NULL END AS normalized_value,
  CASE WHEN row.field_path LIKE '%.date' THEN 'date' WHEN row.field_path ~ '\\.(net|tax|total|requested)$' THEN 'money' ELSE 'text' END AS value_type,
  CASE row.disposition WHEN 'CONFIRMED_CANDIDATE' THEN 'VERIFIED'
   WHEN 'CORRECTED' THEN 'CORRECTED' WHEN 'REJECTED' THEN 'UNKNOWN'
   ELSE CASE WHEN row.source_state='CANDIDATE' THEN 'CANDIDATE' ELSE 'UNKNOWN' END END AS state,
  CASE WHEN row.disposition IS NOT NULL AND cardinality(row.decision_evidence)>0
   THEN row.decision_evidence
   ELSE COALESCE((SELECT array_agg(e.id ORDER BY e.block_index,e.id)
    FROM finance.evidence e
    WHERE e.tenant_id=t AND e.company_id=c AND e.ocr_run_id=row.ocr_id
     AND e.page=row.source_page AND e.block_index=ANY(row.block_indexes)),ARRAY[]::uuid[]) END AS evidence_ids,
  row.schema_version AS extractor_version
 FROM current_rows row
 ORDER BY row.id;
END $$;

CREATE OR REPLACE FUNCTION finance.runtime_field_meta(t uuid,c uuid,d uuid,a uuid,field_id uuid)
RETURNS TABLE(path text,value_type text,current_value text)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF field_id IS NULL OR NOT finance.field_access_allowed(t,c,d,a)
 THEN RAISE EXCEPTION 'field metadata forbidden' USING ERRCODE='42501'; END IF;
 RETURN QUERY
 SELECT f.field_path,
  CASE WHEN f.field_path LIKE '%.date' THEN 'date' WHEN f.field_path ~ '\\.(net|tax|total|requested)$' THEN 'money' ELSE 'text' END,
  f.normalized_value
 FROM finance.field_identity fi
 JOIN finance.extracted_field f
  ON (f.tenant_id,f.company_id,f.run_id,f.field_path)=(fi.tenant_id,fi.company_id,fi.run_id,fi.field_path)
 JOIN finance.extraction_run x
  ON (x.tenant_id,x.company_id,x.id)=(f.tenant_id,f.company_id,f.run_id)
 JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(x.tenant_id,x.company_id,x.ocr_run_id)
 JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(o.tenant_id,o.company_id,o.document_id)
 JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(r.tenant_id,r.company_id,r.dossier_id)
 JOIN finance.dossier_revision dr
  ON (dr.tenant_id,dr.company_id,dr.id,dr.number)=(ds.tenant_id,ds.company_id,r.revision_id,ds.revision)
 WHERE fi.tenant_id=t AND fi.company_id=c AND fi.id=field_id AND ds.id=d
  AND o.revision_id=dr.id
  AND NOT EXISTS(SELECT 1 FROM finance.ocr_run newer
   WHERE newer.tenant_id=o.tenant_id AND newer.company_id=o.company_id
    AND newer.document_id=o.document_id AND newer.revision_id=o.revision_id
    AND (newer.created_at>o.created_at OR (newer.created_at=o.created_at AND newer.id>o.id)))
 LIMIT 1;
END $$;

CREATE OR REPLACE FUNCTION finance.runtime_field_receipt(t uuid,c uuid,d uuid,a uuid,decision_id uuid)
RETURNS TABLE(
 id uuid,path text,raw_value text,normalized_value text,value_type text,state text,
 evidence_ids uuid[],extractor_version text,dossier_version bigint
)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF decision_id IS NULL OR NOT finance.field_access_allowed(t,c,d,a)
 THEN RAISE EXCEPTION 'field receipt forbidden' USING ERRCODE='42501'; END IF;
 RETURN QUERY
 SELECT fi.id,f.field_path,
  (SELECT string_agg(e.excerpt,' | ' ORDER BY e.block_index,e.id)
   FROM finance.evidence e JOIN finance.extraction_run x2
    ON (x2.tenant_id,x2.company_id,x2.ocr_run_id)=(e.tenant_id,e.company_id,e.ocr_run_id)
   WHERE x2.tenant_id=f.tenant_id AND x2.company_id=f.company_id AND x2.id=f.run_id
    AND e.page=f.source_page AND e.block_index=ANY(f.block_indexes)),
  q.normalized_value,
  CASE WHEN f.field_path LIKE '%.date' THEN 'date' WHEN f.field_path ~ '\\.(net|tax|total|requested)$' THEN 'money' ELSE 'text' END,
  CASE q.disposition WHEN 'CONFIRMED_CANDIDATE' THEN 'VERIFIED'
   WHEN 'CORRECTED' THEN 'CORRECTED' ELSE 'UNKNOWN' END,
  q.evidence_ids,x.schema_version,q.dossier_version
 FROM finance.field_decision q
 JOIN finance.field_identity fi
  ON (fi.tenant_id,fi.company_id,fi.run_id,fi.field_path)=(q.tenant_id,q.company_id,q.run_id,q.field_path)
 JOIN finance.extracted_field f
  ON (f.tenant_id,f.company_id,f.run_id,f.field_path)=(fi.tenant_id,fi.company_id,fi.run_id,fi.field_path)
 JOIN finance.extraction_run x
  ON (x.tenant_id,x.company_id,x.id)=(f.tenant_id,f.company_id,f.run_id)
 WHERE q.tenant_id=t AND q.company_id=c AND q.dossier_id=d AND q.actor_id=a AND q.id=decision_id
 LIMIT 1;
END $$;
