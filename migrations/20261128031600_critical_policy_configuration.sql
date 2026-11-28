-- Source-clause bound critical field mapping, reviewed as part of policy publication.
GRANT SELECT,INSERT ON finance.policy_critical_field TO finance_policy_governance;
GRANT UPDATE(criticality_complete) ON finance.policy_version TO finance_policy_governance;
CREATE POLICY governance_critical_read ON finance.policy_critical_field FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY governance_critical_insert ON finance.policy_critical_field FOR INSERT TO finance_policy_governance
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND EXISTS(
 SELECT 1 FROM finance.policy_version pv WHERE (pv.tenant_id,pv.company_id,pv.id)=(policy_critical_field.tenant_id,policy_critical_field.company_id,policy_critical_field.policy_id)
 AND pv.state='DRAFT' AND pv.author_id=finance.scope_actor()));
CREATE FUNCTION finance.critical_registry_freeze() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP<>'INSERT' THEN RAISE EXCEPTION 'immutable critical field' USING ERRCODE='23514'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.policy_version pv WHERE (pv.tenant_id,pv.company_id,pv.id)=(NEW.tenant_id,NEW.company_id,NEW.policy_id) AND pv.state='DRAFT')
 THEN RAISE EXCEPTION 'critical registry frozen' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER critical_registry_freeze BEFORE INSERT OR UPDATE OR DELETE ON finance.policy_critical_field FOR EACH ROW EXECUTE FUNCTION finance.critical_registry_freeze();
CREATE FUNCTION finance.runtime_policy_create_configured(t uuid,c uuid,a uuid,pid uuid,idem text,pname text,kind text,starts date,ends date,source uuid,
 clause_keys text[],ev_ids uuid[],check_keys text[],doc_kinds text[],required_flags boolean[],cmd uuid,corr uuid,critical_keys text[],critical_paths text[])
RETURNS TABLE(id uuid,company_id uuid,version bigint,name text,state text,effective_from text,effective_to text,hash text)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE digest bytea; previous bytea; source_hash bytea; payload jsonb; affected bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR pid IS NULL OR source IS NULL OR cmd IS NULL OR corr IS NULL
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
 OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$' OR pname IS NULL OR octet_length(pname) NOT BETWEEN 1 AND 200 OR btrim(pname)='' OR pname ~ '[[:cntrl:]]'
 OR kind IS NULL OR kind NOT IN ('PAYMENT','ADVANCE','SETTLEMENT') OR starts IS NULL OR ends IS NULL OR starts>=ends
 OR clause_keys IS NULL OR ev_ids IS NULL OR check_keys IS NULL OR doc_kinds IS NULL OR required_flags IS NULL
 OR critical_keys IS NULL OR critical_paths IS NULL OR cardinality(critical_keys)<>cardinality(critical_paths) OR cardinality(critical_paths)>200
 OR EXISTS(SELECT 1 FROM unnest(critical_keys) k WHERE k IS NULL OR NOT(k=ANY(clause_keys)))
 OR EXISTS(SELECT 1 FROM unnest(critical_paths) k WHERE k IS NULL OR k !~ '^[a-z][a-z0-9_]{0,63}[.][a-z][a-z0-9_]{0,63} 1 AND 200 OR cardinality(clause_keys)<>cardinality(ev_ids)
 OR cardinality(check_keys) NOT BETWEEN 1 AND 200 OR cardinality(check_keys)<>cardinality(doc_kinds) OR cardinality(check_keys)<>cardinality(required_flags)
 OR array_position(required_flags,NULL::boolean) IS NOT NULL OR NOT (true=ANY(required_flags))
 OR EXISTS(SELECT 1 FROM unnest(clause_keys) k WHERE k IS NULL OR k !~ '^[A-Za-z0-9._-]{1,64}$')
 OR EXISTS(SELECT 1 FROM unnest(doc_kinds) k WHERE k IS NULL OR k !~ '^[A-Z_]{1,64}$')
 OR EXISTS(SELECT 1 FROM unnest(check_keys) k WHERE k IS NULL OR NOT (k=ANY(clause_keys)))
 OR cardinality(clause_keys)<>(SELECT count(DISTINCT k) FROM unnest(clause_keys) k)
 OR cardinality(ev_ids)<>(SELECT count(DISTINCT e) FROM unnest(ev_ids) e)
 OR cardinality(doc_kinds)<>(SELECT count(DISTINCT k) FROM unnest(doc_kinds) k)
 THEN RAISE EXCEPTION 'invalid policy draft' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='POLICY_ADMIN' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'policy maker required' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.policy.command/v1/'||t::text||'/'||c::text||'/'||a::text||'/policy.create/'||idem,0));
 SELECT b.sha256_digest INTO source_hash FROM finance.document_blob b
 JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id)=(b.tenant_id,b.company_id,b.id)
 JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(b.tenant_id,b.company_id,b.id)
 WHERE (b.tenant_id,b.company_id,b.id)=(t,c,source) AND b.scan_state='CLEAN' AND u.state='FINALIZED' AND u.actor_id=a;
 IF NOT FOUND THEN RAISE EXCEPTION 'clean authorized policy source required' USING ERRCODE='42501'; END IF;
 digest:=sha256(convert_to(jsonb_build_array(pname,kind,starts,ends,source,encode(source_hash,'hex'),clause_keys,ev_ids,check_keys,doc_kinds,required_flags,critical_keys,critical_paths)::text,'UTF8'));
 SELECT ir.request_hash,ir.result_payload INTO previous,payload FROM finance.idempotency_record ir
 WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'policy.create',idem);
 IF FOUND THEN
  IF previous<>digest THEN RAISE EXCEPTION 'policy replay conflict' USING ERRCODE='23514'; END IF;
 ELSE
  PERFORM finance.prototype_policy_lock(t,c,kind);
  INSERT INTO finance.policy_version(tenant_id,company_id,id,dossier_kind,author_id,effective_from,effective_until,source_digest,display_name)
   VALUES(t,c,pid,kind,a,starts,ends,source_hash,pname);
  INSERT INTO finance.policy_clause(tenant_id,company_id,policy_id,id,clause_key,excerpt,source_digest,source_page)
   SELECT t,c,pid,e.id,z.clause_key,e.excerpt,source_hash,e.page
   FROM unnest(clause_keys,ev_ids) z(clause_key,evidence_id)
   JOIN finance.evidence e ON (e.tenant_id,e.company_id,e.id)=(t,c,z.evidence_id)
   JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(e.tenant_id,e.company_id,e.ocr_run_id)
   WHERE o.document_id=source AND o.source_digest=source_hash;
  GET DIAGNOSTICS affected=ROW_COUNT;
  IF affected<>cardinality(clause_keys) THEN RAISE EXCEPTION 'policy evidence mismatch' USING ERRCODE='23514'; END IF;
  INSERT INTO finance.checklist_item(tenant_id,company_id,policy_id,id,clause_id,document_kind,mandatory)
   SELECT t,c,pid,gen_random_uuid(),cl.id,z.document_kind,z.mandatory
   FROM unnest(check_keys,doc_kinds,required_flags) z(clause_key,document_kind,mandatory)
   JOIN finance.policy_clause cl ON (cl.tenant_id,cl.company_id,cl.policy_id,cl.clause_key)=(t,c,pid,z.clause_key);
  GET DIAGNOSTICS affected=ROW_COUNT;
  IF affected<>cardinality(check_keys) THEN RAISE EXCEPTION 'checklist mapping mismatch' USING ERRCODE='23514'; END IF;
  INSERT INTO finance.policy_critical_field(tenant_id,company_id,policy_id,clause_id,field_path)
  SELECT t,c,pid,cl.id,z.field_path
  FROM unnest(critical_keys,critical_paths) z(clause_key,field_path)
  JOIN finance.policy_clause cl ON (cl.tenant_id,cl.company_id,cl.policy_id,cl.clause_key)=(t,c,pid,z.clause_key);
  GET DIAGNOSTICS affected=ROW_COUNT;
  IF affected<>cardinality(critical_paths) THEN RAISE EXCEPTION 'critical field mapping mismatch' USING ERRCODE='23514'; END IF;
  payload:=jsonb_build_object('id',pid,'company_id',c,'version',1,'name',pname,'state','DRAFT',
   'effective_from',to_char(starts,'YYYY-MM-DD')||'T00:00:00Z','effective_to',to_char(ends,'YYYY-MM-DD')||'T00:00:00Z','hash',encode(source_hash,'hex'));
  INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
   VALUES(t,c,cmd,a,cmd,corr,pid,'POLICY_CREATED');
  INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES(t,c,cmd,cmd,pid,'POLICY_CREATED');
  INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_version,result_payload)
   VALUES(t,c,a,'policy.create',idem,digest,pid,1,payload);
 END IF;
 RETURN QUERY SELECT x.id,x.company_id,x.version,x.name,x.state,x.effective_from,x.effective_to,x.hash
 FROM jsonb_to_record(payload) AS x(id uuid,company_id uuid,version bigint,name text,state text,effective_from text,effective_to text,hash text);
END $$;
ALTER FUNCTION finance.runtime_policy_create_configured(uuid,uuid,uuid,uuid,text,text,text,date,date,uuid,text[],uuid[],text[],text[],boolean[],uuid,uuid,text[],text[]) OWNER TO finance_policy_governance;
REVOKE ALL ON FUNCTION finance.runtime_policy_create_configured(uuid,uuid,uuid,uuid,text,text,text,date,date,uuid,text[],uuid[],text[],text[],boolean[],uuid,uuid,text[],text[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_policy_create_configured(uuid,uuid,uuid,uuid,text,text,text,date,date,uuid,text[],uuid[],text[],text[],boolean[],uuid,uuid,text[],text[]) TO finance_runtime;
)
 OR cardinality(critical_paths)<>(SELECT count(DISTINCT k) FROM unnest(critical_paths) k)
 OR cardinality(clause_keys) NOT BETWEEN 1 AND 200 OR cardinality(clause_keys)<>cardinality(ev_ids)
 OR cardinality(check_keys) NOT BETWEEN 1 AND 200 OR cardinality(check_keys)<>cardinality(doc_kinds) OR cardinality(check_keys)<>cardinality(required_flags)
 OR array_position(required_flags,NULL::boolean) IS NOT NULL OR NOT (true=ANY(required_flags))
 OR EXISTS(SELECT 1 FROM unnest(clause_keys) k WHERE k IS NULL OR k !~ '^[A-Za-z0-9._-]{1,64}$')
 OR EXISTS(SELECT 1 FROM unnest(doc_kinds) k WHERE k IS NULL OR k !~ '^[A-Z_]{1,64}$')
 OR EXISTS(SELECT 1 FROM unnest(check_keys) k WHERE k IS NULL OR NOT (k=ANY(clause_keys)))
 OR cardinality(clause_keys)<>(SELECT count(DISTINCT k) FROM unnest(clause_keys) k)
 OR cardinality(ev_ids)<>(SELECT count(DISTINCT e) FROM unnest(ev_ids) e)
 OR cardinality(doc_kinds)<>(SELECT count(DISTINCT k) FROM unnest(doc_kinds) k)
 THEN RAISE EXCEPTION 'invalid policy draft' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='POLICY_ADMIN' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'policy maker required' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.policy.command/v1/'||t::text||'/'||c::text||'/'||a::text||'/policy.create/'||idem,0));
 SELECT b.sha256_digest INTO source_hash FROM finance.document_blob b
 JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id)=(b.tenant_id,b.company_id,b.id)
 JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(b.tenant_id,b.company_id,b.id)
 WHERE (b.tenant_id,b.company_id,b.id)=(t,c,source) AND b.scan_state='CLEAN' AND u.state='FINALIZED' AND u.actor_id=a;
 IF NOT FOUND THEN RAISE EXCEPTION 'clean authorized policy source required' USING ERRCODE='42501'; END IF;
 digest:=sha256(convert_to(jsonb_build_array(pname,kind,starts,ends,source,encode(source_hash,'hex'),clause_keys,ev_ids,check_keys,doc_kinds,required_flags)::text,'UTF8'));
 SELECT ir.request_hash,ir.result_payload INTO previous,payload FROM finance.idempotency_record ir
 WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'policy.create',idem);
 IF FOUND THEN
  IF previous<>digest THEN RAISE EXCEPTION 'policy replay conflict' USING ERRCODE='23514'; END IF;
 ELSE
  PERFORM finance.prototype_policy_lock(t,c,kind);
  INSERT INTO finance.policy_version(tenant_id,company_id,id,dossier_kind,author_id,effective_from,effective_until,source_digest,display_name)
   VALUES(t,c,pid,kind,a,starts,ends,source_hash,pname);
  INSERT INTO finance.policy_clause(tenant_id,company_id,policy_id,id,clause_key,excerpt,source_digest,source_page)
   SELECT t,c,pid,e.id,z.clause_key,e.excerpt,source_hash,e.page
   FROM unnest(clause_keys,ev_ids) z(clause_key,evidence_id)
   JOIN finance.evidence e ON (e.tenant_id,e.company_id,e.id)=(t,c,z.evidence_id)
   JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(e.tenant_id,e.company_id,e.ocr_run_id)
   WHERE o.document_id=source AND o.source_digest=source_hash;
  GET DIAGNOSTICS affected=ROW_COUNT;
  IF affected<>cardinality(clause_keys) THEN RAISE EXCEPTION 'policy evidence mismatch' USING ERRCODE='23514'; END IF;
  INSERT INTO finance.checklist_item(tenant_id,company_id,policy_id,id,clause_id,document_kind,mandatory)
   SELECT t,c,pid,gen_random_uuid(),cl.id,z.document_kind,z.mandatory
   FROM unnest(check_keys,doc_kinds,required_flags) z(clause_key,document_kind,mandatory)
   JOIN finance.policy_clause cl ON (cl.tenant_id,cl.company_id,cl.policy_id,cl.clause_key)=(t,c,pid,z.clause_key);
  GET DIAGNOSTICS affected=ROW_COUNT;
  IF affected<>cardinality(check_keys) THEN RAISE EXCEPTION 'checklist mapping mismatch' USING ERRCODE='23514'; END IF;
  payload:=jsonb_build_object('id',pid,'company_id',c,'version',1,'name',pname,'state','DRAFT',
   'effective_from',to_char(starts,'YYYY-MM-DD')||'T00:00:00Z','effective_to',to_char(ends,'YYYY-MM-DD')||'T00:00:00Z','hash',encode(source_hash,'hex'));
  INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
   VALUES(t,c,cmd,a,cmd,corr,pid,'POLICY_CREATED');
  INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES(t,c,cmd,cmd,pid,'POLICY_CREATED');
  INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_version,result_payload)
   VALUES(t,c,a,'policy.create',idem,digest,pid,1,payload);
 END IF;
 RETURN QUERY SELECT x.id,x.company_id,x.version,x.name,x.state,x.effective_from,x.effective_to,x.hash
 FROM jsonb_to_record(payload) AS x(id uuid,company_id uuid,version bigint,name text,state text,effective_from text,effective_to text,hash text);
END $$;
ALTER FUNCTION finance.runtime_policy_create_configured(uuid,uuid,uuid,uuid,text,text,text,date,date,uuid,text[],uuid[],text[],text[],boolean[],uuid,uuid,text[],text[]) OWNER TO finance_policy_governance;
REVOKE ALL ON FUNCTION finance.runtime_policy_create_configured(uuid,uuid,uuid,uuid,text,text,text,date,date,uuid,text[],uuid[],text[],text[],boolean[],uuid,uuid,text[],text[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_policy_create_configured(uuid,uuid,uuid,uuid,text,text,text,date,date,uuid,text[],uuid[],text[],text[],boolean[],uuid,uuid,text[],text[]) TO finance_runtime;

CREATE OR REPLACE FUNCTION finance.runtime_policy_change(t uuid,c uuid,a uuid,pid uuid,v bigint,op text,idem text,why text,new_name text,day date,new_id uuid,cmd uuid,corr uuid)
RETURNS TABLE(id uuid,company_id uuid,version bigint,name text,state text,effective_from text,effective_to text,hash text)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE p finance.policy_version%ROWTYPE; target_id uuid; action_name text; role_name text; digest bytea; previous bytea; payload jsonb;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR pid IS NULL OR v IS NULL OR new_id IS NULL OR cmd IS NULL OR corr IS NULL
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
 OR op IS NULL OR op NOT IN ('policy.submit','policy.publish','policy.retire','policy.revoke','policy.revise')
 OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$' OR why IS NULL OR octet_length(why) NOT BETWEEN 1 AND 2048 OR btrim(why)='' OR why ~ '[[:cntrl:]]'
 OR (op='policy.revise' AND (day IS NULL OR new_name IS NULL OR octet_length(new_name) NOT BETWEEN 1 AND 200 OR btrim(new_name)='' OR new_name ~ '[[:cntrl:]]'))
 OR (op='policy.publish' AND day IS NULL)
 OR (op<>'policy.revise' AND new_name IS NOT NULL)
 OR (op NOT IN ('policy.revise','policy.publish') AND day IS NOT NULL)
 THEN RAISE EXCEPTION 'invalid policy command' USING ERRCODE='22023'; END IF;
 role_name:=CASE WHEN op IN ('policy.submit','policy.revise') THEN 'POLICY_ADMIN' ELSE 'FINANCE_APPROVER' END;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
   AND m.role=role_name AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'policy role required' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.policy.command/v1/'||t::text||'/'||c::text||'/'||a::text||'/'||op||'/'||idem,0));
 SELECT pv.* INTO p FROM finance.policy_version pv WHERE (pv.tenant_id,pv.company_id,pv.id)=(t,c,pid);
 IF NOT FOUND OR (role_name='POLICY_ADMIN' AND p.author_id<>a) OR (role_name='FINANCE_APPROVER' AND p.author_id=a)
 THEN RAISE EXCEPTION 'policy authority required' USING ERRCODE='42501'; END IF;
 PERFORM finance.prototype_policy_lock(t,c,p.dossier_kind);
 SELECT pv.* INTO STRICT p FROM finance.policy_version pv WHERE (pv.tenant_id,pv.company_id,pv.id)=(t,c,pid) FOR UPDATE;
 digest:=sha256(convert_to(jsonb_build_array(op,pid,v,why,new_name,day)::text,'UTF8'));
 SELECT ir.request_hash,ir.result_payload INTO previous,payload FROM finance.idempotency_record ir
 WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,op,idem);
 IF FOUND THEN
  IF previous<>digest THEN RAISE EXCEPTION 'policy replay conflict' USING ERRCODE='23514'; END IF;
 ELSE
  IF p.aggregate_version<>v THEN RAISE EXCEPTION 'stale policy version' USING ERRCODE='23514'; END IF;
  target_id:=pid;
  IF op='policy.revise' THEN
   IF p.state='IN_REVIEW' OR day>=p.effective_until THEN RAISE EXCEPTION 'invalid revision' USING ERRCODE='23514'; END IF;
   INSERT INTO finance.policy_version(tenant_id,company_id,id,dossier_kind,author_id,effective_from,effective_until,source_digest,display_name,supersedes_id)
    VALUES(t,c,new_id,p.dossier_kind,a,day,p.effective_until,p.source_digest,new_name,pid);
   INSERT INTO finance.policy_clause(tenant_id,company_id,policy_id,id,clause_key,excerpt,source_digest,source_page)
    SELECT t,c,new_id,cl.id,cl.clause_key,cl.excerpt,cl.source_digest,cl.source_page FROM finance.policy_clause cl
    WHERE (cl.tenant_id,cl.company_id,cl.policy_id)=(t,c,pid);
   INSERT INTO finance.checklist_item(tenant_id,company_id,policy_id,id,clause_id,document_kind,mandatory)
    SELECT t,c,new_id,ci.id,ci.clause_id,ci.document_kind,ci.mandatory FROM finance.checklist_item ci
    WHERE (ci.tenant_id,ci.company_id,ci.policy_id)=(t,c,pid);
   INSERT INTO finance.policy_critical_field(tenant_id,company_id,policy_id,clause_id,field_path)
   SELECT t,c,new_id,cf.clause_id,cf.field_path FROM finance.policy_critical_field cf
   WHERE (cf.tenant_id,cf.company_id,cf.policy_id)=(t,c,pid);
   target_id:=new_id; action_name:='POLICY_REVISED';
  ELSIF op='policy.submit' THEN
   IF NOT EXISTS(SELECT 1 FROM finance.rule_version r WHERE (r.tenant_id,r.company_id,r.policy_id)=(t,c,pid) AND r.state='DRAFT')
   OR NOT EXISTS(SELECT 1 FROM finance.form_version f WHERE (f.tenant_id,f.company_id,f.policy_id)=(t,c,pid) AND f.state='DRAFT')
   OR NOT EXISTS(SELECT 1 FROM finance.template_version f WHERE (f.tenant_id,f.company_id,f.policy_id)=(t,c,pid) AND f.state='DRAFT')
   OR NOT EXISTS(SELECT 1 FROM finance.workflow_version w WHERE (w.tenant_id,w.company_id,w.policy_id)=(t,c,pid) AND w.state='DRAFT')
   THEN RAISE EXCEPTION 'incomplete policy bindings' USING ERRCODE='23514'; END IF;
   PERFORM finance.prototype_policy_transition(t,c,pid,a,'IN_REVIEW'); action_name:='POLICY_SUBMITTED';
  ELSIF op='policy.publish' THEN
   IF day IS DISTINCT FROM p.effective_from THEN RAISE EXCEPTION 'effective date differs from reviewed version' USING ERRCODE='23514'; END IF;
   PERFORM finance.prototype_policy_transition(t,c,pid,a,'PUBLISHED');
   -- The independent checker approves the frozen critical mapping together
   -- with the policy. Empty legacy registries remain incomplete.
   UPDATE finance.policy_version pv SET criticality_complete=true
   WHERE (pv.tenant_id,pv.company_id,pv.id)=(t,c,pid) AND EXISTS(
   SELECT 1 FROM finance.policy_critical_field cf WHERE (cf.tenant_id,cf.company_id,cf.policy_id)=(t,c,pid));
   action_name:='POLICY_PUBLISHED';
  ELSIF op='policy.retire' THEN
   PERFORM finance.prototype_policy_transition(t,c,pid,a,'RETIRED'); action_name:='POLICY_RETIRED';
  ELSE
   PERFORM finance.prototype_policy_transition(t,c,pid,a,'REVOKED'); action_name:='POLICY_REVOKED';
  END IF;
  SELECT jsonb_build_object('id',pv.id,'company_id',pv.company_id,'version',pv.aggregate_version,
   'name',COALESCE(pv.display_name,pv.dossier_kind||' policy'),'state',pv.state,
   'effective_from',to_char(pv.effective_from,'YYYY-MM-DD')||'T00:00:00Z',
   'effective_to',to_char(pv.effective_until,'YYYY-MM-DD')||'T00:00:00Z','hash',encode(pv.source_digest,'hex'))
  INTO payload FROM finance.policy_version pv WHERE (pv.tenant_id,pv.company_id,pv.id)=(t,c,target_id);
  INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
   VALUES(t,c,cmd,a,cmd,corr,target_id,action_name);
  INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
   VALUES(t,c,cmd,cmd,target_id,action_name);
  INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_version,result_payload)
   VALUES(t,c,a,op,idem,digest,target_id,(payload->>'version')::bigint,payload);
 END IF;
 RETURN QUERY SELECT x.id,x.company_id,x.version,x.name,x.state,x.effective_from,x.effective_to,x.hash
 FROM jsonb_to_record(payload) AS x(id uuid,company_id uuid,version bigint,name text,state text,effective_from text,effective_to text,hash text);
END $$;
ALTER FUNCTION finance.runtime_policy_change(uuid,uuid,uuid,uuid,bigint,text,text,text,text,date,uuid,uuid,uuid) OWNER TO finance_policy_governance;
REVOKE ALL ON FUNCTION finance.runtime_policy_change(uuid,uuid,uuid,uuid,bigint,text,text,text,text,date,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_policy_change(uuid,uuid,uuid,uuid,bigint,text,text,text,text,date,uuid,uuid,uuid) TO finance_runtime;

