-- Isolated synthetic invoice PNG worker only. Never process external or real records.
-- The existing document and extraction schemas are explicitly synthetic-only.
GRANT SELECT ON finance.outbox TO finance_workflow_writer;
CREATE POLICY ocr_workflow_outbox_read ON finance.outbox FOR SELECT TO finance_workflow_writer
 USING(finance.worker_scope_allowed(tenant_id,company_id));
GRANT INSERT ON finance.ocr_run,finance.evidence,finance.extraction_run,finance.extracted_field,finance.field_identity TO finance_workflow_writer;
CREATE POLICY ocr_workflow_run_insert ON finance.ocr_run FOR INSERT TO finance_workflow_writer
 WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
CREATE POLICY ocr_workflow_evidence_insert ON finance.evidence FOR INSERT TO finance_workflow_writer
 WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
CREATE POLICY ocr_workflow_evidence_read ON finance.evidence FOR SELECT TO finance_workflow_writer
 USING(finance.worker_scope_allowed(tenant_id,company_id));
GRANT SELECT ON finance.evidence TO finance_workflow_writer;
CREATE POLICY ocr_workflow_extraction_insert ON finance.extraction_run FOR INSERT TO finance_workflow_writer
 WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
CREATE POLICY ocr_workflow_field_insert ON finance.extracted_field FOR INSERT TO finance_workflow_writer
 WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
CREATE POLICY ocr_workflow_field_identity_insert ON finance.field_identity FOR INSERT TO finance_workflow_writer
 WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));

CREATE FUNCTION finance.synthetic_invoice_ocr_source(t uuid,c uuid,j uuid,f bigint,w uuid)
RETURNS TABLE(document_id uuid,revision_id uuid,generation bigint,source_digest bytea,content bytea)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT b.id,r.revision_id,b.generation,b.sha256_digest,b.synthetic_bytes
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
 AND q.state='LEASED' AND q.lease_until>statement_timestamp() AND o.event_type='OCR_REQUESTED'
 AND b.scan_state='CLEAN' AND b.storage_backend='SYNTHETIC_DB_ONLY' AND b.mime='image/png'
 AND u.state='FINALIZED' AND u.generation=b.generation AND u.document_kind='INVOICE'
 AND d.status IN ('DRAFT','PROCESSING','NEEDS_INPUT','REVIEWABLE')
$$;
ALTER FUNCTION finance.synthetic_invoice_ocr_source(uuid,uuid,uuid,bigint,uuid) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.synthetic_invoice_ocr_source(uuid,uuid,uuid,bigint,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.synthetic_invoice_ocr_source(uuid,uuid,uuid,bigint,uuid) TO finance_job_worker;

CREATE FUNCTION finance.complete_synthetic_invoice_ocr(t uuid,c uuid,j uuid,f bigint,w uuid,
 doc uuid,rev uuid,g bigint,source bytea,model bytea,config bytea,blocks jsonb,fields jsonb)
RETURNS uuid LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE jobrow finance.job%ROWTYPE; b finance.document_blob%ROWTYPE; p jsonb; entry jsonb; idx integer:=0;
 rect integer[]; ref_ids integer[]; page_no integer; field_count integer:=0; run_id uuid:=j;
 payload_digest bytea;
BEGIN
 IF t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR NOT finance.worker_scope_allowed(t,c)
 THEN RAISE EXCEPTION 'worker authority' USING ERRCODE='42501'; END IF;
 IF current_setting('transaction_isolation')<>'read committed'
 OR t IS NULL OR c IS NULL OR j IS NULL OR w IS NULL OR doc IS NULL OR rev IS NULL OR f IS NULL OR g IS NULL
 OR source IS NULL OR octet_length(source)<>32 OR model IS NULL OR octet_length(model)<>32
 OR config IS NULL OR octet_length(config)<>32
 OR jsonb_typeof(blocks) IS DISTINCT FROM 'array' OR jsonb_array_length(blocks)>10000
 OR octet_length(blocks::text)>2097152 OR jsonb_typeof(fields) IS DISTINCT FROM 'array'
 OR jsonb_array_length(fields)<>6 OR octet_length(fields::text)>65536
 THEN RAISE EXCEPTION 'invalid synthetic OCR payload' USING ERRCODE='23514'; END IF;
 SELECT * INTO jobrow FROM finance.job WHERE tenant_id=t AND company_id=c AND id=j FOR UPDATE;
 IF NOT FOUND OR jobrow.state<>'LEASED' OR jobrow.fence<>f OR jobrow.leased_by<>w OR jobrow.lease_until<=clock_timestamp()
 THEN RAISE EXCEPTION 'stale OCR lease' USING ERRCODE='40001'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.outbox WHERE tenant_id=t AND company_id=c AND id=j AND resource_id=doc AND event_type='OCR_REQUESTED')
 THEN RAISE EXCEPTION 'wrong OCR job source' USING ERRCODE='23514'; END IF;
 SELECT * INTO b FROM finance.document_blob WHERE tenant_id=t AND company_id=c AND id=doc;
 IF NOT FOUND OR b.generation<>g OR b.sha256_digest<>source OR b.storage_backend<>'SYNTHETIC_DB_ONLY'
 OR b.scan_state<>'CLEAN' OR b.mime<>'image/png'
 OR NOT EXISTS(SELECT 1 FROM finance.synthetic_invoice_ocr_source(t,c,j,f,w) s
   WHERE s.document_id=doc AND s.revision_id=rev AND s.generation=g AND s.source_digest=source)
 THEN RAISE EXCEPTION 'wrong OCR source' USING ERRCODE='23514'; END IF;
 payload_digest:=sha256(convert_to(jsonb_build_array(blocks,fields)::text,'UTF8'));
 INSERT INTO finance.ocr_run(tenant_id,company_id,id,document_id,revision_id,source_digest,generation,job_id,fence,engine,model_digest,config_digest,page_count,result_digest)
 VALUES(t,c,run_id,doc,rev,source,g,j,f,'TESSERACT_SYNTHETIC',model,config,1,payload_digest);
 FOR p IN SELECT value FROM jsonb_array_elements(blocks) LOOP
  IF jsonb_typeof(p) IS DISTINCT FROM 'object' OR jsonb_typeof(p->'rect') IS DISTINCT FROM 'array'
  OR jsonb_array_length(p->'rect')<>4 OR jsonb_typeof(p->'text') IS DISTINCT FROM 'string'
  THEN RAISE EXCEPTION 'invalid OCR block' USING ERRCODE='23514'; END IF;
  SELECT array_agg(x::integer ORDER BY ord) INTO rect FROM jsonb_array_elements_text(p->'rect') WITH ORDINALITY AS a(x,ord);
  INSERT INTO finance.evidence(tenant_id,company_id,id,ocr_run_id,page,block_index,x0,y0,x1,y1,excerpt,confidence)
  VALUES(t,c,gen_random_uuid(),run_id,1,idx,rect[1],rect[2],rect[3],rect[4],p->>'text',(p->>'confidence')::integer);
  idx:=idx+1;
 END LOOP;
 INSERT INTO finance.extraction_run VALUES(t,c,run_id,run_id,'synthetic.rows.v1');
 FOR entry IN SELECT value FROM jsonb_array_elements(fields) LOOP
  IF jsonb_typeof(entry) IS DISTINCT FROM 'object' OR jsonb_typeof(entry->'indexes') IS DISTINCT FROM 'array'
  OR jsonb_array_length(entry->'indexes')>16 THEN RAISE EXCEPTION 'invalid extracted field' USING ERRCODE='23514'; END IF;
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
 IF field_count<>6 OR jobrow.lease_until<=clock_timestamp() THEN RAISE EXCEPTION 'incomplete result or expired lease' USING ERRCODE='23514'; END IF;
 UPDATE finance.job_attempt SET state='SUCCEEDED',finished_at=clock_timestamp() WHERE tenant_id=t AND company_id=c AND job_id=j AND fence=f AND worker_id=w AND state='ACTIVE';
 IF NOT FOUND THEN RAISE EXCEPTION 'stale OCR attempt' USING ERRCODE='23514'; END IF;
 UPDATE finance.job SET state='SUCCEEDED',leased_by=NULL,lease_until=NULL WHERE tenant_id=t AND company_id=c AND id=j
  AND fence=f AND leased_by=w AND state='LEASED' AND lease_until>clock_timestamp();
 IF NOT FOUND THEN RAISE EXCEPTION 'stale OCR lease' USING ERRCODE='40001'; END IF;
 INSERT INTO finance.inbox(tenant_id,company_id,event_id,fence) VALUES(t,c,j,f);
 RETURN run_id;
END $$;
REVOKE ALL ON FUNCTION finance.complete_synthetic_invoice_ocr(uuid,uuid,uuid,bigint,uuid,uuid,uuid,bigint,bytea,bytea,bytea,jsonb,jsonb) FROM PUBLIC;

ALTER FUNCTION finance.complete_synthetic_invoice_ocr(uuid,uuid,uuid,bigint,uuid,uuid,uuid,bigint,bytea,bytea,bytea,jsonb,jsonb) OWNER TO finance_workflow_writer;
GRANT EXECUTE ON FUNCTION finance.complete_synthetic_invoice_ocr(uuid,uuid,uuid,bigint,uuid,uuid,uuid,bigint,bytea,bytea,bytea,jsonb,jsonb) TO finance_job_worker;
