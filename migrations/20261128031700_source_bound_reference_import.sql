-- Require source-bound normalized financial reference imports; never fabricate a paid amount.
ALTER TABLE finance.reference_snapshot ADD COLUMN source_document_id uuid,
 ADD COLUMN import_payload jsonb,
 ADD COLUMN import_evidence uuid[],
 ADD FOREIGN KEY(tenant_id,company_id,source_document_id) REFERENCES finance.document_blob(tenant_id,company_id,id),
 ADD CHECK((source_document_id IS NULL)=(import_payload IS NULL) AND (import_payload IS NULL)=(import_evidence IS NULL)),
 ADD CHECK(import_payload IS NULL OR (jsonb_typeof(import_payload)='array' AND jsonb_array_length(import_payload)<=10000));
GRANT SELECT ON finance.ocr_run,finance.document_blob TO finance_reference_writer;
GRANT INSERT ON finance.financial_resource TO finance_reference_writer;
CREATE POLICY reference_source_blob_read ON finance.document_blob FOR SELECT TO finance_reference_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY reference_source_ocr_read ON finance.ocr_run FOR SELECT TO finance_reference_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY reference_resource_verified_insert ON finance.financial_resource FOR INSERT TO finance_reference_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND coverage_complete AND EXISTS(
 SELECT 1 FROM finance.reference_snapshot r WHERE (r.tenant_id,r.company_id,r.id)=(financial_resource.tenant_id,financial_resource.company_id,financial_resource.snapshot_id)
 AND r.state='DRAFT' AND r.maker_id<>finance.scope_actor() AND r.source_document_id IS NOT NULL));
-- Remove access to the legacy metadata-only function which inserted a fictitious payment.
REVOKE EXECUTE ON FUNCTION finance.runtime_create_reference(uuid,uuid,uuid,uuid,text,timestamptz,text,text,uuid,uuid,uuid) FROM finance_runtime;
CREATE FUNCTION finance.runtime_create_reference_bound(
 t uuid,c uuid,a uuid,doc uuid,kind text,as_of_ts timestamptz,coverage text,idem text,ref_id uuid,cmd uuid,corr uuid,fresh_ts timestamptz,records jsonb
) RETURNS TABLE(id uuid,state text,attempt bigint,progress bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE digest bytea; existing finance.idempotency_record%ROWTYPE; payload jsonb; fresh timestamptz; source_hash bytea; item jsonb; all_evidence uuid[];
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR doc IS NULL OR as_of_ts IS NULL OR ref_id IS NULL OR cmd IS NULL OR corr IS NULL
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR kind IS NULL OR kind NOT IN ('VENDOR','CONTRACT','INVOICE','ADVANCE','BUDGET','PAYMENT')
 OR records IS NULL OR jsonb_typeof(records) IS DISTINCT FROM 'array' OR jsonb_array_length(records)>10000 OR octet_length(records::text)>8388608
 OR as_of_ts>clock_timestamp() OR fresh_ts IS NULL OR fresh_ts<=as_of_ts
 OR kind NOT IN ('CONTRACT','INVOICE','ADVANCE','BUDGET','PAYMENT') OR jsonb_array_length(records)=0
  OR coverage IS NULL OR coverage NOT IN ('COMPLETE','PARTIAL','UNKNOWN')
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid reference creation request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role IN ('REQUESTER','POLICY_ADMIN') AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'requester or policy admin required' USING ERRCODE='42501'; END IF;

 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reference.create/v2/'||t::text||'/'||c::text||'/'||a::text||'/'||idem,0));
 SELECT b.sha256_digest INTO source_hash FROM finance.document_blob b JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id)=(b.tenant_id,b.company_id,b.id)
 WHERE (b.tenant_id,b.company_id,b.id)=(t,c,doc) AND u.actor_id=a AND u.state='FINALIZED' AND b.scan_state='CLEAN' AND b.generation=u.generation AND b.sha256_digest=u.declared_sha256;
 IF NOT FOUND THEN RAISE EXCEPTION 'authorized clean reference source required' USING ERRCODE='42501'; END IF;
 digest:=sha256(convert_to(jsonb_build_array(doc,encode(source_hash,'hex'),kind,coverage,as_of_ts,fresh_ts,records)::text,'UTF8'));
 SELECT * INTO existing FROM finance.idempotency_record
  WHERE (tenant_id,company_id,actor_id,operation_id,key)=(t,c,a,'reference.create',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'id')::uuid,
   existing.result_payload->>'state',
   (existing.result_payload->>'attempt')::bigint,
   (existing.result_payload->>'progress')::bigint;
  RETURN;
 END IF;

 -- Every mapped row cites at least one block from the immutable source.
 IF EXISTS(SELECT 1 FROM jsonb_array_elements(records) item
 WHERE jsonb_typeof(item) IS DISTINCT FROM 'object' OR jsonb_typeof(item->'evidence_ids') IS DISTINCT FROM 'array'
 OR jsonb_array_length(item->'evidence_ids') NOT BETWEEN 1 AND 50)
 THEN RAISE EXCEPTION 'reference mapping requires evidence' USING ERRCODE='23514'; END IF;
 SELECT coalesce(array_agg(DISTINCT x::uuid ORDER BY x::uuid),ARRAY[]::uuid[]) INTO all_evidence
 FROM jsonb_array_elements(records) item CROSS JOIN LATERAL jsonb_array_elements_text(item->'evidence_ids') x;
 IF cardinality(all_evidence)>50 OR EXISTS(SELECT 1 FROM unnest(all_evidence) x WHERE NOT EXISTS(
 SELECT 1 FROM finance.evidence e JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(e.tenant_id,e.company_id,e.ocr_run_id)
 WHERE (e.tenant_id,e.company_id,e.id)=(t,c,x) AND o.document_id=doc AND o.source_digest=source_hash))
 THEN RAISE EXCEPTION 'reference mapping evidence not bound to source' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.reference_snapshot(tenant_id,company_id,id,source_system,external_version,source_digest,as_of,fresh_until,maker_id,state,source_document_id,import_payload,import_evidence)
 VALUES(t,c,ref_id,'MANUAL_API',kind||':'||coverage||':'||ref_id::text,source_hash,as_of_ts,fresh_ts,a,'DRAFT',doc,records,all_evidence);
 IF kind IN ('CONTRACT','INVOICE','ADVANCE','BUDGET') THEN
  INSERT INTO finance.source_coverage(tenant_id,company_id,snapshot_id,resource_kind,complete,watermark)
  VALUES(t,c,ref_id,kind,coverage='COMPLETE','manual:'||ref_id::text);
 END IF;
 -- Generic normalized resource rows are materialized only by the independent
 -- checker. The source identity is explicit and never synthesized from amounts.
 FOR item IN SELECT value FROM jsonb_array_elements(records) LOOP
  IF kind='PAYMENT' THEN
   IF (SELECT count(*) FROM jsonb_object_keys(item)) NOT BETWEEN 8 AND 9
   OR (item ? 'allocations' AND item->'allocations'<>'null'::jsonb AND jsonb_typeof(item->'allocations') IS DISTINCT FROM 'object')
   OR NOT (item ?& ARRAY['source_key','currency','amount','reservation_id','event_kind','reverses','occurred_at','evidence_ids'])
   OR item->>'source_key' IS NULL OR octet_length(item->>'source_key') NOT BETWEEN 1 AND 128 OR item->>'source_key' ~ '[[:cntrl:]]'
   OR item->>'currency' IS NULL OR item->>'currency' !~ '^[A-Z]{3}$'
   OR item->>'amount' IS NULL OR item->>'amount' !~ '^[0-9]{1,20}([.][0-9]{1,8})?$' OR (item->>'amount')::numeric<=0
   OR item->>'reservation_id' IS NULL OR (item->>'reservation_id')::uuid='00000000-0000-0000-0000-000000000000'
   OR item->>'event_kind' IS NULL OR item->>'event_kind' NOT IN ('CONSUME','REVERSE')
   OR ((item->>'event_kind'='REVERSE') IS DISTINCT FROM (item->>'reverses' IS NOT NULL))
   OR item->>'occurred_at' IS NULL OR (item->>'occurred_at')::timestamptz>as_of_ts
   THEN RAISE EXCEPTION 'invalid payment observation row' USING ERRCODE='23514'; END IF;
   CONTINUE;
  END IF;
  IF item->>'source_key' IS NULL OR octet_length(item->>'source_key') NOT BETWEEN 1 AND 128 OR item->>'source_key' ~ '[[:cntrl:]]'
  OR item->>'currency' IS NULL OR item->>'currency' !~ '^[A-Z]{3}$'
  OR item->>'cap' IS NULL OR item->>'cap' !~ '^[0-9]{1,20}([.][0-9]{1,8})?$'
  OR item->>'consumed' IS NULL OR item->>'consumed' !~ '^[0-9]{1,20}([.][0-9]{1,8})?$'
  OR (item->>'consumed')::numeric>(item->>'cap')::numeric
  OR kind NOT IN ('CONTRACT','INVOICE','ADVANCE','BUDGET')
  OR (SELECT count(*) FROM jsonb_object_keys(item))<>6 OR NOT COALESCE(finance.validate_reference_details(kind,item,(as_of_ts AT TIME ZONE 'UTC')::date),false)
  THEN RAISE EXCEPTION 'invalid normalized resource row' USING ERRCODE='23514'; END IF;
 END LOOP;
 IF (SELECT count(DISTINCT item->>'source_key') FROM jsonb_array_elements(records) item)<>jsonb_array_length(records)
 THEN RAISE EXCEPTION 'duplicate normalized resource identity' USING ERRCODE='23514'; END IF;
 payload:=jsonb_build_object('id',ref_id::text,'state','QUEUED','attempt',0,'progress',0);
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,ref_id,'REFERENCE_CREATED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,ref_id,cmd,ref_id,'REFERENCE_CREATED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'reference.create',idem,digest,ref_id,payload);
 RETURN QUERY SELECT ref_id,'QUEUED'::text,0::bigint,0::bigint;
END $$;
ALTER FUNCTION finance.runtime_create_reference_bound(uuid,uuid,uuid,uuid,text,timestamptz,text,text,uuid,uuid,uuid,timestamptz,jsonb) OWNER TO finance_reference_writer;
REVOKE ALL ON FUNCTION finance.runtime_create_reference_bound(uuid,uuid,uuid,uuid,text,timestamptz,text,text,uuid,uuid,uuid,timestamptz,jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_create_reference_bound(uuid,uuid,uuid,uuid,text,timestamptz,text,text,uuid,uuid,uuid,timestamptz,jsonb) TO finance_runtime;


CREATE OR REPLACE FUNCTION finance.runtime_verify_reference(
 t uuid,c uuid,a uuid,ref_id uuid,v bigint,reason text,ev_ids uuid[],idem text,cmd uuid,corr uuid
) RETURNS TABLE(id uuid,kind text,source_key text,as_of text,coverage text,state text,evidence_ids uuid[],version bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE
 digest bytea; existing finance.idempotency_record%ROWTYPE; r finance.reference_snapshot%ROWTYPE;
 rkind text; rcov text; asof_txt text; payload jsonb; ev_count bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR ref_id IS NULL OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR reason IS NULL OR octet_length(reason) NOT BETWEEN 1 AND 4096 OR btrim(reason)='' OR reason ~ '[[:cntrl:]]'
  OR ev_ids IS NULL OR cardinality(ev_ids) NOT BETWEEN 1 AND 50 OR array_position(ev_ids,NULL::uuid) IS NOT NULL
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid reference verification request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='FINANCE_APPROVER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'finance approver required' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to('reference.verify:'||ref_id::text||':'||v::text||':'||reason||':'||array_to_string(ev_ids,','),'UTF8'));
 SELECT * INTO existing FROM finance.idempotency_record
  WHERE (tenant_id,company_id,actor_id,operation_id,key)=(t,c,a,'reference.verify',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>ref_id THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'id')::uuid,
   existing.result_payload->>'kind',
   existing.result_payload->>'source_key',
   existing.result_payload->>'as_of',
   existing.result_payload->>'coverage',
   existing.result_payload->>'state',
   ARRAY(SELECT jsonb_array_elements_text(existing.result_payload->'evidence_ids')::uuid),
   (existing.result_payload->>'version')::bigint;
  RETURN;
 END IF;

 SELECT count(DISTINCT e.id)::bigint INTO ev_count FROM finance.evidence e
  WHERE e.tenant_id=t AND e.company_id=c AND e.id=ANY(ev_ids);
 IF ev_count<>(SELECT count(DISTINCT x) FROM unnest(ev_ids) x) THEN
  RAISE EXCEPTION 'unknown evidence id' USING ERRCODE='22023';
 END IF;

 SELECT * INTO r FROM finance.reference_snapshot WHERE (tenant_id,company_id,id)=(t,c,ref_id) FOR UPDATE;
 IF NOT FOUND OR r.source_system<>'MANUAL_API' OR v<>0 OR r.state<>'DRAFT' THEN
  RAISE EXCEPTION 'reference not verifiable or version conflict' USING ERRCODE='23505';
 END IF;
 IF r.maker_id=a THEN
  RAISE EXCEPTION 'independent verifier required' USING ERRCODE='42501';
 END IF;

 rkind:=split_part(r.external_version,':',1);
 rcov:=split_part(r.external_version,':',2);
 IF r.source_document_id IS NULL OR r.import_payload IS NULL OR jsonb_array_length(r.import_payload)=0 OR r.fresh_until<=clock_timestamp() OR r.as_of>clock_timestamp()
 OR rkind NOT IN ('CONTRACT','INVOICE','ADVANCE','BUDGET','PAYMENT')
 OR NOT EXISTS(SELECT 1 FROM finance.document_blob b WHERE (b.tenant_id,b.company_id,b.id)=(t,c,r.source_document_id) AND b.scan_state='CLEAN' AND b.sha256_digest=r.source_digest)
 OR NOT (ev_ids <@ r.import_evidence) OR EXISTS(SELECT 1 FROM jsonb_array_elements(r.import_payload) item WHERE NOT EXISTS(SELECT 1 FROM jsonb_array_elements_text(item->'evidence_ids') ev WHERE ev::uuid=ANY(ev_ids))) OR cardinality(ev_ids)<>(SELECT count(DISTINCT x) FROM unnest(ev_ids) x)
 THEN RAISE EXCEPTION 'reference lacks verified normalized source material' USING ERRCODE='23514'; END IF;
 IF rcov<>'COMPLETE' THEN
  RAISE EXCEPTION 'incomplete coverage cannot be verified' USING ERRCODE='23505';
 END IF;

 -- Independent checker reviews the exact frozen normalized rows and source
 -- evidence. A logical resource cannot be silently replaced while holds exist.
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reference/v1/'||t::text||'/'||c::text,0));
 IF rkind='PAYMENT' THEN
  -- Payment events are independently approved observations, not payment orders.
  INSERT INTO finance.payment_ref(tenant_id,company_id,snapshot_id,id,external_key,currency,amount,state,occurred_at,reservation_id,event_kind,reverses_key,line_allocations)
  SELECT t,c,ref_id,gen_random_uuid(),item->>'source_key',item->>'currency',(item->>'amount')::numeric,
   CASE WHEN item->>'event_kind'='REVERSE' THEN 'PARTIALLY_REVERSED' ELSE 'PARTIALLY_PAID' END,
   (item->>'occurred_at')::timestamptz,(item->>'reservation_id')::uuid,item->>'event_kind',item->>'reverses',nullif(item->'allocations','null'::jsonb)
  FROM jsonb_array_elements(r.import_payload) item;
 ELSE
 IF EXISTS(SELECT 1 FROM jsonb_array_elements(r.import_payload) item WHERE NOT COALESCE(finance.validate_reference_details(rkind,item,(r.as_of AT TIME ZONE 'UTC')::date),false)) THEN RAISE EXCEPTION 'invalid typed reference' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.financial_resource(tenant_id,company_id,id,currency,cap,consumed,coverage_complete,fresh_until,snapshot_id,resource_kind,source_key,reference_details)
 SELECT t,c,gen_random_uuid(),item->>'currency',(item->>'cap')::numeric,(item->>'consumed')::numeric,true,r.fresh_until,ref_id,rkind,jsonb_build_array('MANUAL_API',item->>'source_key')::text,(item->'details')||jsonb_build_object('frozen_on',(r.as_of AT TIME ZONE 'UTC')::date)
 FROM jsonb_array_elements(r.import_payload) item;
 END IF;
 UPDATE finance.reference_snapshot SET state='APPROVED',checker_id=a
  WHERE (tenant_id,company_id,id)=(t,c,ref_id) AND state='DRAFT';

 asof_txt:=to_char(r.as_of AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"');
 payload:=jsonb_build_object(
  'id',ref_id::text,'kind',rkind,'source_key',r.external_version,'as_of',asof_txt,
  'coverage',rcov,'state','VERIFIED','evidence_ids',to_jsonb(ev_ids),'version',1
 );
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,ref_id,'REFERENCE_VERIFIED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,cmd,cmd,ref_id,'REFERENCE_VERIFIED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'reference.verify',idem,digest,ref_id,payload);
 RETURN QUERY SELECT ref_id,rkind,r.external_version,asof_txt,rcov,'VERIFIED'::text,ev_ids,1::bigint;
END $$;
ALTER FUNCTION finance.runtime_verify_reference(uuid,uuid,uuid,uuid,bigint,text,uuid[],text,uuid,uuid) OWNER TO finance_reference_writer;
REVOKE ALL ON FUNCTION finance.runtime_verify_reference(uuid,uuid,uuid,uuid,bigint,text,uuid[],text,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_verify_reference(uuid,uuid,uuid,uuid,bigint,text,uuid[],text,uuid,uuid) TO finance_runtime;


