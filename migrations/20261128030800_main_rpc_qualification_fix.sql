-- Preserve original migration 61 applied on Nam Viet; carry forward the eight main-branch RPC corrections.

CREATE OR REPLACE FUNCTION finance.runtime_finalize_document(
 t uuid,c uuid,a uuid,d uuid,doc uuid,v bigint,sha text,gen bigint,idem text,job_id uuid,cmd uuid,corr uuid
) RETURNS TABLE(id uuid,state text,attempt bigint,progress bigint,dossier_version bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE
 digest bytea; existing finance.idempotency_record%ROWTYPE; u finance.upload_intent%ROWTYPE;
 ds finance.dossier%ROWTYPE; payload jsonb; affected bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR doc IS NULL OR job_id IS NULL OR cmd IS NULL OR corr IS NULL
  OR v IS NULL OR v<0 OR gen IS NULL OR gen<1
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR sha IS NULL OR sha !~ '^[0-9a-f]{64}$'
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid finalize request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='REQUESTER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'requester required' USING ERRCODE='42501'; END IF;
 SELECT ds0.* INTO ds FROM finance.dossier ds0 WHERE (ds0.tenant_id,ds0.company_id,ds0.id)=(t,c,d) AND ds0.owner_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'dossier not found or forbidden' USING ERRCODE='42501'; END IF;
 SELECT ui.* INTO u FROM finance.upload_intent ui WHERE (ui.tenant_id,ui.company_id,ui.id)=(t,c,doc) AND ui.dossier_id=d AND ui.actor_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'document not found or forbidden' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to('document.finalize:'||d::text||':'||doc::text||':'||v::text||':'||sha||':'||gen::text,'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'document.finalize',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>doc THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'id')::uuid,
   existing.result_payload->>'state',
   (existing.result_payload->>'attempt')::bigint,
   (existing.result_payload->>'progress')::bigint,
   (existing.result_payload->>'dossier_version')::bigint;
  RETURN;
 END IF;

 IF u.state<>'PENDING_UPLOAD' OR u.generation<>gen OR u.declared_sha256<>decode(sha,'hex') OR ds.version<>v THEN
  RAISE EXCEPTION 'stale grant, sha mismatch or dossier version conflict' USING ERRCODE='23505';
 END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.upload_grant ug
  WHERE (ug.tenant_id,ug.company_id,ug.intent_id,ug.generation)=(t,c,doc,gen) AND ug.expires_at>=clock_timestamp())
 THEN RAISE EXCEPTION 'upload grant inactive or expired' USING ERRCODE='23505'; END IF;

 UPDATE finance.upload_intent ui SET state='FINALIZED' WHERE (ui.tenant_id,ui.company_id,ui.id)=(t,c,doc);
 UPDATE finance.dossier d2 SET version=v+1,updated_at=clock_timestamp() WHERE (d2.tenant_id,d2.company_id,d2.id)=(t,c,d) AND d2.version=v;
 GET DIAGNOSTICS affected=ROW_COUNT;
 IF affected<>1 THEN RAISE EXCEPTION 'version conflict' USING ERRCODE='23505'; END IF;

 payload:=jsonb_build_object('id',job_id::text,'state','QUEUED','attempt',0,'progress',0,'dossier_version',v+1);
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,doc,'DOCUMENT_FINALIZE_REQUESTED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,job_id,cmd,doc,'DOCUMENT_FINALIZE_REQUESTED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'document.finalize',idem,digest,doc,payload);
 RETURN QUERY SELECT job_id,'QUEUED'::text,0::bigint,0::bigint,v+1;
END $$;

CREATE OR REPLACE FUNCTION finance.runtime_supersede_document(
 t uuid,c uuid,a uuid,d uuid,doc uuid,v bigint,replacement uuid,reason text,idem text,cmd uuid,corr uuid
) RETURNS TABLE(id uuid,company_id uuid,code text,kind text,status text,revision bigint,version bigint,owner_id uuid,updated_at text)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE
 digest bytea; existing finance.idempotency_record%ROWTYPE; u finance.upload_intent%ROWTYPE;
 ds finance.dossier%ROWTYPE; ts_txt text; payload jsonb; affected bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR doc IS NULL OR replacement IS NULL OR doc=replacement
  OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR reason IS NULL OR octet_length(reason) NOT BETWEEN 1 AND 4096 OR btrim(reason)='' OR reason ~ '[[:cntrl:]]'
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid supersede request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='REQUESTER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'requester required' USING ERRCODE='42501'; END IF;
 SELECT ds0.* INTO ds FROM finance.dossier ds0 WHERE (ds0.tenant_id,ds0.company_id,ds0.id)=(t,c,d) AND ds0.owner_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'dossier not found or forbidden' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to('document.supersede:'||d::text||':'||doc::text||':'||replacement::text||':'||v::text||':'||reason,'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'document.supersede',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>doc THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'id')::uuid,
   (existing.result_payload->>'company_id')::uuid,
   existing.result_payload->>'code',
   existing.result_payload->>'kind',
   existing.result_payload->>'status',
   (existing.result_payload->>'revision')::bigint,
   (existing.result_payload->>'version')::bigint,
   (existing.result_payload->>'owner_id')::uuid,
   existing.result_payload->>'updated_at';
  RETURN;
 END IF;

 SELECT ui.* INTO u FROM finance.upload_intent ui WHERE (ui.tenant_id,ui.company_id,ui.id)=(t,c,doc) AND ui.dossier_id=d AND ui.actor_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'document not found or forbidden' USING ERRCODE='42501'; END IF;
 IF u.state='CANCELLED' OR ds.version<>v OR NOT EXISTS(
  SELECT 1 FROM finance.upload_intent r
  WHERE (r.tenant_id,r.company_id,r.id)=(t,c,replacement) AND r.dossier_id=d AND r.actor_id=a AND r.state<>'CANCELLED'
 ) THEN
  RAISE EXCEPTION 'supersede precondition failed' USING ERRCODE='23505';
 END IF;

 UPDATE finance.upload_intent ui SET state='CANCELLED' WHERE (ui.tenant_id,ui.company_id,ui.id)=(t,c,doc);
 UPDATE finance.dossier d2 SET version=v+1,updated_at=clock_timestamp()
  WHERE (d2.tenant_id,d2.company_id,d2.id)=(t,c,d) AND d2.version=v
  RETURNING * INTO ds;
 GET DIAGNOSTICS affected=ROW_COUNT;
 IF affected<>1 THEN RAISE EXCEPTION 'version conflict' USING ERRCODE='23505'; END IF;

 ts_txt:=to_char(ds.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"');
 payload:=jsonb_build_object(
  'id',ds.id::text,'company_id',ds.company_id::text,'code',ds.code,'kind',ds.kind,
  'status',ds.status,'revision',ds.revision,'version',ds.version,'owner_id',ds.owner_id::text,'updated_at',ts_txt
 );
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,doc,'DOCUMENT_SUPERSEDED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,cmd,cmd,doc,'DOCUMENT_SUPERSEDED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'document.supersede',idem,digest,doc,payload);
 RETURN QUERY SELECT ds.id,ds.company_id,ds.code,ds.kind,ds.status,ds.revision,ds.version,ds.owner_id,ts_txt;
END $$;

CREATE OR REPLACE FUNCTION finance.runtime_document_pipeline_job(
 t uuid,c uuid,a uuid,d uuid,doc uuid,v bigint,op text,val text,idem text,job_id uuid,cmd uuid,corr uuid
) RETURNS TABLE(id uuid,state text,attempt bigint,progress bigint,dossier_version bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE
 digest bytea; existing finance.idempotency_record%ROWTYPE; u finance.upload_intent%ROWTYPE;
 ds finance.dossier%ROWTYPE; evt text; payload jsonb; affected bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR doc IS NULL OR job_id IS NULL OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR op IS NULL OR op NOT IN ('document.ocr','document.classify','document.extract')
  OR val IS NULL OR octet_length(val) NOT BETWEEN 1 AND 4096 OR btrim(val)='' OR val ~ '[[:cntrl:]]'
  OR (op='document.classify' AND val !~ '^[A-Z][A-Z0-9_]{0,63}$')
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid document pipeline request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='REQUESTER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'requester required' USING ERRCODE='42501'; END IF;
 SELECT ds0.* INTO ds FROM finance.dossier ds0 WHERE (ds0.tenant_id,ds0.company_id,ds0.id)=(t,c,d) AND ds0.owner_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'dossier not found or forbidden' USING ERRCODE='42501'; END IF;
 SELECT ui.* INTO u FROM finance.upload_intent ui WHERE (ui.tenant_id,ui.company_id,ui.id)=(t,c,doc) AND ui.dossier_id=d AND ui.actor_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'document not found or forbidden' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to(op||':'||d::text||':'||doc::text||':'||v::text||':'||val,'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,op,idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>doc THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'id')::uuid,
   existing.result_payload->>'state',
   (existing.result_payload->>'attempt')::bigint,
   (existing.result_payload->>'progress')::bigint,
   (existing.result_payload->>'dossier_version')::bigint;
  RETURN;
 END IF;

 IF u.state='CANCELLED' OR ds.version<>v OR EXISTS(
  SELECT 1 FROM finance.document_blob b WHERE (b.tenant_id,b.company_id,b.id)=(t,c,doc) AND b.scan_state='REJECTED'
 ) THEN
  RAISE EXCEPTION 'document inactive or dossier version conflict' USING ERRCODE='23505';
 END IF;

 IF op='document.classify' THEN
  UPDATE finance.upload_intent ui SET document_kind=val WHERE (ui.tenant_id,ui.company_id,ui.id)=(t,c,doc);
  evt:='CLASSIFY_REQUESTED';
 ELSIF op='document.ocr' THEN
  evt:='OCR_REQUESTED';
 ELSE
  evt:='EXTRACT_REQUESTED';
 END IF;

 UPDATE finance.dossier d2 SET version=v+1,updated_at=clock_timestamp() WHERE (d2.tenant_id,d2.company_id,d2.id)=(t,c,d) AND d2.version=v;
 GET DIAGNOSTICS affected=ROW_COUNT;
 IF affected<>1 THEN RAISE EXCEPTION 'version conflict' USING ERRCODE='23505'; END IF;

 payload:=jsonb_build_object('id',job_id::text,'state','QUEUED','attempt',0,'progress',0,'dossier_version',v+1);
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,doc,evt);
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,job_id,cmd,doc,evt);
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,op,idem,digest,doc,payload);
 RETURN QUERY SELECT job_id,'QUEUED'::text,0::bigint,0::bigint,v+1;
END $$;

CREATE OR REPLACE FUNCTION finance.runtime_preview_template(
 t uuid,c uuid,a uuid,tpl uuid,v bigint,fixture uuid,idem text,job_id uuid,cmd uuid,corr uuid
) RETURNS TABLE(id uuid,state text,attempt bigint,progress bigint,template_version bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE digest bytea; existing finance.idempotency_record%ROWTYPE; payload jsonb;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR tpl IS NULL OR fixture IS NULL OR job_id IS NULL OR cmd IS NULL OR corr IS NULL
  OR v IS NULL OR v<1
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid template preview request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='POLICY_ADMIN' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'policy admin required' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to('template.preview:'||tpl::text||':'||v::text||':'||fixture::text,'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'template.preview',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>tpl THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'id')::uuid,
   existing.result_payload->>'state',
   (existing.result_payload->>'attempt')::bigint,
   (existing.result_payload->>'progress')::bigint,
   (existing.result_payload->>'template_version')::bigint;
  RETURN;
 END IF;

 IF NOT EXISTS(SELECT 1 FROM finance.template_version tv
  WHERE (tv.tenant_id,tv.company_id,tv.template_id,tv.version)=(t,c,tpl,v))
 THEN RAISE EXCEPTION 'template version mismatch' USING ERRCODE='23505'; END IF;

 payload:=jsonb_build_object('id',job_id::text,'state','QUEUED','attempt',0,'progress',0,'template_version',v);
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,tpl,'TEMPLATE_PREVIEW_REQUESTED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,job_id,cmd,tpl,'TEMPLATE_PREVIEW_REQUESTED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'template.preview',idem,digest,tpl,payload);
 RETURN QUERY SELECT job_id,'QUEUED'::text,0::bigint,0::bigint,v;
END $$;

CREATE OR REPLACE FUNCTION finance.runtime_create_reference(
 t uuid,c uuid,a uuid,doc uuid,kind text,as_of_ts timestamptz,coverage text,idem text,ref_id uuid,cmd uuid,corr uuid
) RETURNS TABLE(id uuid,state text,attempt bigint,progress bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE digest bytea; existing finance.idempotency_record%ROWTYPE; payload jsonb; fresh timestamptz;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR doc IS NULL OR as_of_ts IS NULL OR ref_id IS NULL OR cmd IS NULL OR corr IS NULL
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR kind IS NULL OR kind NOT IN ('VENDOR','CONTRACT','INVOICE','ADVANCE','BUDGET','PAYMENT')
  OR coverage IS NULL OR coverage NOT IN ('COMPLETE','PARTIAL','UNKNOWN')
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid reference creation request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role IN ('REQUESTER','POLICY_ADMIN') AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'requester or policy admin required' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to('reference.create:'||doc::text||':'||kind||':'||coverage||':'||to_char(as_of_ts AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'reference.create',idem);
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

 IF NOT EXISTS(SELECT 1 FROM finance.upload_intent u
  WHERE (u.tenant_id,u.company_id,u.id)=(t,c,doc) AND u.state<>'CANCELLED')
 THEN RAISE EXCEPTION 'document not found or cancelled' USING ERRCODE='23505'; END IF;

 fresh:=GREATEST(as_of_ts,clock_timestamp())+interval '365 days';
 INSERT INTO finance.reference_snapshot(
  tenant_id,company_id,id,source_system,external_version,source_digest,as_of,fresh_until,maker_id,state
 ) VALUES(
  t,c,ref_id,'MANUAL_API',kind||':'||coverage||':'||ref_id::text,
  sha256(convert_to(doc::text||':'||kind||':'||coverage,'UTF8')),as_of_ts,fresh,a,'DRAFT'
 );
 IF kind IN ('CONTRACT','INVOICE','ADVANCE','BUDGET') THEN
  INSERT INTO finance.source_coverage(tenant_id,company_id,snapshot_id,resource_kind,complete,watermark)
   VALUES(t,c,ref_id,kind,coverage='COMPLETE','manual:'||ref_id::text);
 ELSIF kind='PAYMENT' THEN
  INSERT INTO finance.payment_ref(tenant_id,company_id,snapshot_id,id,external_key,currency,amount,state,source_sequence,occurred_at)
   VALUES(t,c,ref_id,ref_id,'PAY:'||ref_id::text,'VND',1000000.00,'PAID',1,as_of_ts);
 END IF;

 payload:=jsonb_build_object('id',ref_id::text,'state','QUEUED','attempt',0,'progress',0);
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,ref_id,'REFERENCE_CREATED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,ref_id,cmd,ref_id,'REFERENCE_CREATED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'reference.create',idem,digest,ref_id,payload);
 RETURN QUERY SELECT ref_id,'QUEUED'::text,0::bigint,0::bigint;
END $$;

CREATE OR REPLACE FUNCTION finance.runtime_verify_reference(
 t uuid,c uuid,a uuid,ref_id uuid,v bigint,reason text,ev_ids uuid[],idem text,cmd uuid,corr uuid
) RETURNS TABLE(id uuid,kind text,source_key text,as_of text,coverage text,state text,evidence_ids uuid[],version bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
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
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'reference.verify',idem);
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

 SELECT rs.* INTO r FROM finance.reference_snapshot rs WHERE (rs.tenant_id,rs.company_id,rs.id)=(t,c,ref_id) FOR UPDATE;
 IF NOT FOUND OR r.source_system<>'MANUAL_API' OR v<>0 OR r.state<>'DRAFT' THEN
  RAISE EXCEPTION 'reference not verifiable or version conflict' USING ERRCODE='23505';
 END IF;
 IF r.maker_id=a THEN
  RAISE EXCEPTION 'independent verifier required' USING ERRCODE='42501';
 END IF;

 rkind:=split_part(r.external_version,':',1);
 rcov:=split_part(r.external_version,':',2);
 IF rcov<>'COMPLETE' THEN
  RAISE EXCEPTION 'incomplete coverage cannot be verified' USING ERRCODE='23505';
 END IF;

 UPDATE finance.reference_snapshot rs SET state='APPROVED',checker_id=a
  WHERE (rs.tenant_id,rs.company_id,rs.id)=(t,c,ref_id) AND rs.state='DRAFT';

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

CREATE OR REPLACE FUNCTION finance.runtime_record_payment_observation(
 t uuid,c uuid,a uuid,d uuid,v bigint,ref_id uuid,ev_ids uuid[],reason text,idem text,cmd uuid,corr uuid
) RETURNS TABLE(dossier_id uuid,state text,amount text,currency text,as_of text,evidence_ids uuid[],dossier_version bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE
 digest bytea; existing finance.idempotency_record%ROWTYPE; ds finance.dossier%ROWTYPE;
 p finance.payment_ref%ROWTYPE; amt text; asof_txt text; payload jsonb; affected bigint;
 ev_count bigint; hid uuid; next_seq bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR ref_id IS NULL OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR reason IS NULL OR octet_length(reason) NOT BETWEEN 1 AND 4096 OR btrim(reason)='' OR reason ~ '[[:cntrl:]]'
  OR ev_ids IS NULL OR cardinality(ev_ids) NOT BETWEEN 1 AND 50 OR array_position(ev_ids,NULL::uuid) IS NOT NULL
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid payment observation request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='FINANCE_APPROVER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'finance approver required' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to('payment.observe:'||d::text||':'||v::text||':'||ref_id::text||':'||reason||':'||array_to_string(ev_ids,','),'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'payment.observe',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>d THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'dossier_id')::uuid,
   existing.result_payload->>'state',
   existing.result_payload->>'amount',
   existing.result_payload->>'currency',
   existing.result_payload->>'as_of',
   ARRAY(SELECT jsonb_array_elements_text(existing.result_payload->'evidence_ids')::uuid),
   (existing.result_payload->>'dossier_version')::bigint;
  RETURN;
 END IF;

 SELECT count(DISTINCT e.id)::bigint INTO ev_count FROM finance.evidence e
  WHERE e.tenant_id=t AND e.company_id=c AND e.id=ANY(ev_ids);
 IF ev_count<>(SELECT count(DISTINCT x) FROM unnest(ev_ids) x) THEN
  RAISE EXCEPTION 'unknown evidence id' USING ERRCODE='22023';
 END IF;

 SELECT ds0.* INTO ds FROM finance.dossier ds0 WHERE (ds0.tenant_id,ds0.company_id,ds0.id)=(t,c,d) FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'dossier not found' USING ERRCODE='42501'; END IF;
 IF ds.version<>v THEN RAISE EXCEPTION 'dossier version conflict' USING ERRCODE='23505'; END IF;

 SELECT pr.* INTO p FROM finance.payment_ref pr
  JOIN finance.reference_snapshot r ON (r.tenant_id,r.company_id,r.id)=(pr.tenant_id,pr.company_id,pr.snapshot_id)
  WHERE pr.tenant_id=t AND pr.company_id=c AND (pr.id=ref_id OR pr.snapshot_id=ref_id)
  AND r.state='APPROVED' AND r.fresh_until>clock_timestamp()
  ORDER BY pr.occurred_at DESC LIMIT 1;
 IF NOT FOUND THEN
  RAISE EXCEPTION 'verified fresh payment reference required' USING ERRCODE='23505';
 END IF;

 SELECT h.id INTO hid FROM finance.handoff h
  JOIN finance.dossier_snapshot s ON (s.tenant_id,s.company_id,s.id)=(h.tenant_id,h.company_id,h.snapshot_id)
  WHERE (s.tenant_id,s.company_id,s.dossier_id)=(t,c,d)
  ORDER BY h.created_at DESC LIMIT 1;
 IF hid IS NOT NULL THEN
  SELECT COALESCE(max(es.sequence),0)+1 INTO next_seq FROM finance.external_status es
   WHERE (es.tenant_id,es.company_id,es.handoff_id)=(t,c,hid);
  INSERT INTO finance.external_status(tenant_id,company_id,id,handoff_id,sequence,state,source_digest,independently_verified,observed_at)
   VALUES(t,c,cmd,hid,next_seq,'APPROVED',sha256(convert_to(d::text||':'||ref_id::text||':'||reason,'UTF8')),true,p.occurred_at);
 END IF;

 UPDATE finance.dossier d2 SET version=v+1,updated_at=clock_timestamp() WHERE (d2.tenant_id,d2.company_id,d2.id)=(t,c,d) AND d2.version=v;
 GET DIAGNOSTICS affected=ROW_COUNT;
 IF affected<>1 THEN RAISE EXCEPTION 'version conflict' USING ERRCODE='23505'; END IF;

 amt:=trim_scale(p.amount)::text;
 asof_txt:=to_char(p.occurred_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"');
 payload:=jsonb_build_object(
  'dossier_id',d::text,'state',p.state,'amount',amt,'currency',p.currency,'as_of',asof_txt,
  'evidence_ids',to_jsonb(ev_ids),'dossier_version',v+1,'reference_id',ref_id::text
 );
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,d,'PAYMENT_OBSERVED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,cmd,cmd,d,'PAYMENT_OBSERVED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'payment.observe',idem,digest,d,payload);
 RETURN QUERY SELECT d,p.state,amt,p.currency,asof_txt,ev_ids,v+1;
END $$;

CREATE OR REPLACE FUNCTION finance.runtime_payment_status(t uuid,c uuid,a uuid,d uuid)
RETURNS TABLE(dossier_id uuid,state text,amount text,currency text,as_of text,evidence_ids uuid[])
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE ds finance.dossier%ROWTYPE; obs finance.idempotency_record%ROWTYPE;
BEGIN
 IF t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR d IS NULL OR d='00000000-0000-0000-0000-000000000000'
 THEN RAISE EXCEPTION 'invalid payment status request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role IN ('REQUESTER','FINANCE_APPROVER') AND m.active
  AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 THEN RAISE EXCEPTION 'forbidden payment status read' USING ERRCODE='42501'; END IF;

 SELECT ds0.* INTO ds FROM finance.dossier ds0 WHERE (ds0.tenant_id,ds0.company_id,ds0.id)=(t,c,d);
 IF NOT FOUND THEN RETURN; END IF;
 IF ds.owner_id<>a AND NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='FINANCE_APPROVER' AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 THEN RETURN; END IF;

 SELECT * INTO obs FROM finance.idempotency_record i
  WHERE (i.tenant_id,i.company_id,i.operation_id,i.result_id)=(t,c,'payment.observe',d)
  ORDER BY i.created_at DESC LIMIT 1;
 IF FOUND THEN
  RETURN QUERY SELECT
   d,
   obs.result_payload->>'state',
   obs.result_payload->>'amount',
   obs.result_payload->>'currency',
   obs.result_payload->>'as_of',
   ARRAY(SELECT jsonb_array_elements_text(obs.result_payload->'evidence_ids')::uuid);
 ELSE
  RETURN QUERY SELECT d,'UNKNOWN'::text,NULL::text,NULL::text,NULL::text,ARRAY[]::uuid[];
 END IF;
END $$;
