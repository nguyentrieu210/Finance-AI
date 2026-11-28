-- Explicit policy-bound reservation; no payment/consume/release command is exposed.
ALTER TABLE finance.reservation ADD COLUMN binding_policy_id uuid,
 ADD COLUMN binding_actor_id uuid, ADD COLUMN binding_key text, ADD COLUMN binding_request_hash bytea,
 ADD COLUMN binding_material_hash bytea, ADD COLUMN binding_version bigint, ADD COLUMN binding_reason text,
 ADD CHECK((binding_policy_id IS NULL AND binding_actor_id IS NULL AND binding_key IS NULL AND binding_request_hash IS NULL AND binding_material_hash IS NULL AND binding_version IS NULL AND binding_reason IS NULL)
 OR (binding_policy_id IS NOT NULL AND binding_actor_id IS NOT NULL AND binding_key IS NOT NULL AND binding_request_hash IS NOT NULL AND octet_length(binding_request_hash)=32 AND binding_material_hash IS NOT NULL AND octet_length(binding_material_hash)=32 AND binding_version IS NOT NULL AND binding_reason IS NOT NULL));
CREATE UNIQUE INDEX reservation_binding_replay ON finance.reservation(tenant_id,company_id,binding_actor_id,binding_key) WHERE binding_key IS NOT NULL;
GRANT SELECT ON finance.financial_resource,finance.reference_snapshot,finance.source_coverage,finance.reservation_line TO finance_workflow_writer;
GRANT INSERT ON finance.reservation,finance.reservation_line TO finance_workflow_writer;
GRANT UPDATE(binding_policy_id,binding_actor_id,binding_key,binding_request_hash,binding_material_hash,binding_version,binding_reason) ON finance.reservation TO finance_workflow_writer;
GRANT UPDATE(coverage_complete) ON finance.financial_resource TO finance_workflow_writer;
GRANT EXECUTE ON FUNCTION finance.prototype_reserve_finance(uuid,uuid,uuid,uuid,text,numeric,uuid[]) TO finance_workflow_writer;
CREATE POLICY bound_resource_read ON finance.financial_resource FOR SELECT TO finance_workflow_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY bound_resource_lock ON finance.financial_resource FOR UPDATE TO finance_workflow_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()) WITH CHECK(false);
CREATE POLICY bound_source_read ON finance.reference_snapshot FOR SELECT TO finance_workflow_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY bound_coverage_read ON finance.source_coverage FOR SELECT TO finance_workflow_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY bound_line_read ON finance.reservation_line FOR SELECT TO finance_workflow_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY bound_line_insert ON finance.reservation_line FOR INSERT TO finance_workflow_writer WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY bound_reservation_insert ON finance.reservation FOR INSERT TO finance_workflow_writer WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY bound_reservation_update ON finance.reservation FOR UPDATE TO finance_workflow_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()) WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE FUNCTION finance.reservation_binding_immutable() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF OLD.binding_policy_id IS NOT NULL AND (OLD.binding_policy_id,OLD.binding_actor_id,OLD.binding_key,OLD.binding_request_hash,OLD.binding_material_hash,OLD.binding_version,OLD.binding_reason)
 IS DISTINCT FROM (NEW.binding_policy_id,NEW.binding_actor_id,NEW.binding_key,NEW.binding_request_hash,NEW.binding_material_hash,NEW.binding_version,NEW.binding_reason)
 THEN RAISE EXCEPTION 'immutable reservation evidence' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER reservation_binding_immutable BEFORE UPDATE ON finance.reservation FOR EACH ROW EXECUTE FUNCTION finance.reservation_binding_immutable();
DO $$ DECLARE expr text; BEGIN
 SELECT pg_get_expr(conbin,conrelid) INTO STRICT expr FROM pg_constraint WHERE conrelid='finance.audit_event'::regclass AND conname='audit_event_action_check';
 EXECUTE 'ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check';
 EXECUTE format('ALTER TABLE finance.audit_event ADD CONSTRAINT audit_event_action_check CHECK ((%s) OR action=%L)',expr,'FINANCE_RESERVATION_BOUND');
END $$;

CREATE FUNCTION finance.bound_finance_material(t uuid,c uuid,d uuid,rev uuid,pid uuid,v bigint,ids uuid[])
RETURNS jsonb LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE gate jsonb; fields jsonb; amount_row jsonb; currency_row jsonb; n numeric; k uuid; fr finance.financial_resource%ROWTYPE; refs jsonb:='[]'; kinds text[]; expected text[]; cur text;
BEGIN
 IF (t,c) IS DISTINCT FROM (finance.scope_tenant(),finance.scope_company()) OR cardinality(ids) NOT BETWEEN 1 AND 32 OR array_position(ids,NULL::uuid) IS NOT NULL THEN RETURN NULL; END IF;
 SELECT pv.finance_gate INTO gate FROM finance.policy_version pv JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(t,c,d)
 WHERE (pv.tenant_id,pv.company_id,pv.id)=(t,c,pid) AND pv.state='PUBLISHED' AND pv.criticality_complete AND pv.dossier_kind=ds.kind
 AND pv.effective_from<=(clock_timestamp() AT TIME ZONE 'UTC')::date AND pv.effective_until>(clock_timestamp() AT TIME ZONE 'UTC')::date;
 IF gate IS NULL THEN RETURN NULL; END IF;
 SELECT jsonb_agg(row.item ORDER BY row.doc,row.path) INTO fields FROM (
 SELECT o.document_id doc,f.field_path path,jsonb_build_object('document',o.document_id,'path',f.field_path,'decision',q.id,'value',q.normalized_value,'evidence',q.evidence_ids,'verified',coalesce(q.disposition IN ('CONFIRMED_CANDIDATE','CORRECTED') AND cardinality(q.evidence_ids)>0,false),'source',encode(b.sha256_digest,'hex')) item
 FROM finance.document_ref dr JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(dr.tenant_id,dr.company_id,dr.id)
 JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id)=(dr.tenant_id,dr.company_id,dr.id)
 JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.document_id,o.revision_id,o.generation)=(dr.tenant_id,dr.company_id,dr.id,dr.revision_id,b.generation)
 JOIN finance.extraction_run x ON (x.tenant_id,x.company_id,x.ocr_run_id)=(o.tenant_id,o.company_id,o.id)
 JOIN finance.extracted_field f ON (f.tenant_id,f.company_id,f.run_id)=(x.tenant_id,x.company_id,x.id)
 LEFT JOIN LATERAL (SELECT q.id,q.disposition,q.normalized_value,q.evidence_ids FROM finance.field_decision q
 WHERE (q.tenant_id,q.company_id,q.run_id,q.field_path,q.dossier_id,q.revision_id)=(f.tenant_id,f.company_id,f.run_id,f.field_path,d,rev)
 AND q.dossier_version<=v ORDER BY q.dossier_version DESC,q.created_at DESC,q.id DESC LIMIT 1) q ON true
 WHERE (dr.tenant_id,dr.company_id,dr.dossier_id,dr.revision_id)=(t,c,d,rev)
 AND b.scan_state='CLEAN' AND u.state='FINALIZED' AND u.generation=b.generation AND b.sha256_digest=u.declared_sha256 AND o.source_digest=b.sha256_digest
 AND f.field_path IN (gate->>'amount_path',gate->>'currency_path')
 AND NOT EXISTS(SELECT 1 FROM finance.ocr_run newer WHERE (newer.tenant_id,newer.company_id,newer.document_id,newer.revision_id)=(o.tenant_id,o.company_id,o.document_id,o.revision_id) AND (newer.created_at,newer.id)>(o.created_at,o.id))
 LIMIT 3) row;
 IF jsonb_array_length(fields) IS DISTINCT FROM 2 THEN RETURN NULL; END IF;
 SELECT value INTO amount_row FROM jsonb_array_elements(fields) WHERE value->>'path'=gate->>'amount_path';
 SELECT value INTO currency_row FROM jsonb_array_elements(fields) WHERE value->>'path'=gate->>'currency_path';
 IF amount_row IS NULL OR currency_row IS NULL OR amount_row->>'verified' IS DISTINCT FROM 'true' OR currency_row->>'verified' IS DISTINCT FROM 'true'
 OR amount_row->>'document' IS DISTINCT FROM currency_row->>'document' OR amount_row->>'value' IS NULL
 OR amount_row->>'value' !~ '^[0-9]{1,20}([.][0-9]{1,8})?$' THEN RETURN NULL; END IF;
 n:=(amount_row->>'value')::numeric; cur:=currency_row->>'value';
 IF n<=0 OR cur IS NULL OR cur !~ '^[A-Z]{3}$' THEN RETURN NULL; END IF;
 SELECT array_agg(k ORDER BY k) INTO expected FROM jsonb_array_elements_text(gate->'resource_kinds') k;
 FOR k IN SELECT DISTINCT z FROM unnest(ids) z ORDER BY z LOOP
  SELECT * INTO fr FROM finance.financial_resource r WHERE (r.tenant_id,r.company_id,r.id)=(t,c,k) FOR UPDATE;
  IF NOT FOUND OR fr.currency<>cur OR NOT fr.coverage_complete OR fr.fresh_until<=clock_timestamp()
  OR NOT EXISTS(SELECT 1 FROM finance.reference_snapshot s JOIN finance.source_coverage sc ON (sc.tenant_id,sc.company_id,sc.snapshot_id)=(s.tenant_id,s.company_id,s.id)
   WHERE (s.tenant_id,s.company_id,s.id)=(t,c,fr.snapshot_id) AND s.state='APPROVED' AND s.source_document_id IS NOT NULL AND s.fresh_until>clock_timestamp() AND sc.resource_kind=fr.resource_kind AND sc.complete)
  THEN RETURN NULL; END IF;
  refs:=refs||jsonb_build_array(jsonb_build_object('id',fr.id,'snapshot',fr.snapshot_id,'kind',fr.resource_kind,'key',fr.source_key));
 END LOOP;
 SELECT array_agg(z->>'kind' ORDER BY z->>'kind') INTO kinds FROM jsonb_array_elements(refs) z;
 IF kinds IS DISTINCT FROM expected OR jsonb_array_length(refs)<>cardinality(ids) THEN RETURN NULL; END IF;
 RETURN jsonb_build_object('policy',pid,'revision',rev,'amount',n,'currency',cur,'fields',fields,'resources',refs);
END $$;
ALTER FUNCTION finance.bound_finance_material(uuid,uuid,uuid,uuid,uuid,bigint,uuid[]) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.bound_finance_material(uuid,uuid,uuid,uuid,uuid,bigint,uuid[]) FROM PUBLIC;

CREATE FUNCTION finance.current_bound_reservation(t uuid,c uuid,d uuid,rev uuid,pid uuid,v bigint)
RETURNS uuid LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE r finance.reservation%ROWTYPE; mat jsonb; candidate uuid;
BEGIN
 SELECT id INTO candidate FROM finance.reservation x WHERE (x.tenant_id,x.company_id,x.revision_id)=(t,c,rev)
 AND x.state='RESERVED' AND x.binding_policy_id=pid ORDER BY x.id LIMIT 1;
 IF candidate IS NULL THEN RETURN NULL; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reservation/v1/'||t::text||'/'||c::text||'/'||candidate::text,0));
 SELECT * INTO r FROM finance.reservation x WHERE (x.tenant_id,x.company_id,x.id)=(t,c,candidate);
 IF r.state<>'RESERVED' OR r.remaining<>r.amount OR r.binding_version>v THEN RETURN NULL; END IF;
 mat:=finance.bound_finance_material(t,c,d,rev,pid,v,r.resource_ids);
 IF mat IS NULL OR sha256(convert_to(mat::text,'UTF8')) IS DISTINCT FROM r.binding_material_hash
 OR (mat->>'amount')::numeric<>r.amount OR mat->>'currency'<>r.currency THEN RETURN NULL; END IF;
 RETURN r.id;
END $$;
ALTER FUNCTION finance.current_bound_reservation(uuid,uuid,uuid,uuid,uuid,bigint) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.current_bound_reservation(uuid,uuid,uuid,uuid,uuid,bigint) FROM PUBLIC;

CREATE FUNCTION finance.runtime_bind_reservation(t uuid,c uuid,a uuid,d uuid,v bigint,pid uuid,ids uuid[],why text,idem text,i uuid,cmd uuid,corr uuid)
RETURNS TABLE(id uuid,dossier_version bigint) LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; prior finance.reservation%ROWTYPE; mat jsonb; digest bytea; sorted uuid[];
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR v IS NULL OR pid IS NULL OR i IS NULL OR cmd IS NULL OR corr IS NULL
 OR (t,c,a) IS DISTINCT FROM (finance.scope_tenant(),finance.scope_company(),finance.scope_actor()) OR ids IS NULL OR cardinality(ids) NOT BETWEEN 1 AND 4
 OR array_position(ids,NULL::uuid) IS NOT NULL OR why IS NULL OR btrim(why)='' OR octet_length(why)>2048 OR why ~ '[[:cntrl:]]'
 OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$' THEN RAISE EXCEPTION 'invalid reservation command' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a) AND m.role='FINANCE_APPROVER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp())) THEN RAISE EXCEPTION 'finance approver required' USING ERRCODE='42501'; END IF;
 SELECT array_agg(DISTINCT z ORDER BY z) INTO sorted FROM unnest(ids) z;
 IF cardinality(sorted)<>cardinality(ids) THEN RAISE EXCEPTION 'duplicate resources' USING ERRCODE='22023'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reservation.bind/v1/'||t::text||'/'||c::text||'/'||a::text||'/'||idem,0));
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO ds FROM finance.dossier WHERE (tenant_id,company_id,id)=(t,c,d) FOR UPDATE;
 IF NOT FOUND OR ds.owner_id=a THEN RAISE EXCEPTION 'independent finance approver required' USING ERRCODE='42501'; END IF;
 digest:=sha256(convert_to(jsonb_build_array(d,v,pid,sorted,why)::text,'UTF8'));
 SELECT * INTO prior FROM finance.reservation x WHERE (x.tenant_id,x.company_id,x.binding_actor_id,x.binding_key)=(t,c,a,idem);
 IF FOUND THEN
  IF prior.binding_request_hash<>digest THEN RAISE EXCEPTION 'reservation replay conflict' USING ERRCODE='23505'; END IF;
  RETURN QUERY SELECT prior.id,prior.binding_version; RETURN;
 END IF;
 IF ds.version<>v OR ds.status NOT IN ('DRAFT','REVIEWABLE') THEN RAISE EXCEPTION 'stale dossier' USING ERRCODE='23514'; END IF;
 SELECT dr.id INTO STRICT rev FROM finance.dossier_revision dr WHERE (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number)=(t,c,d,ds.revision);
 PERFORM finance.prototype_policy_lock(t,c,ds.kind);
 IF pid IS DISTINCT FROM finance.prototype_effective_policy(t,c,ds.kind,(clock_timestamp() AT TIME ZONE 'UTC')::date) THEN RAISE EXCEPTION 'effective policy required' USING ERRCODE='23514'; END IF;
 IF EXISTS(SELECT 1 FROM finance.reservation x WHERE (x.tenant_id,x.company_id,x.revision_id)=(t,c,rev) AND x.state IN ('RESERVED','COMMITTED','PARTIAL')) THEN RAISE EXCEPTION 'existing hold requires reconciliation' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reservation/v1/'||t::text||'/'||c::text||'/'||i::text,0));
 mat:=finance.bound_finance_material(t,c,d,rev,pid,v,sorted);
 IF mat IS NULL THEN RAISE EXCEPTION 'missing verified finance material' USING ERRCODE='23514'; END IF;
 PERFORM finance.reserve_bound_material(t,c,i,rev,mat,sorted);
 UPDATE finance.reservation x SET binding_policy_id=pid,binding_actor_id=a,binding_key=idem,binding_request_hash=digest,binding_material_hash=sha256(convert_to(mat::text,'UTF8')),binding_version=v+1,binding_reason=why WHERE (x.tenant_id,x.company_id,x.id)=(t,c,i) AND x.binding_policy_id IS NULL;
 IF NOT FOUND THEN RAISE EXCEPTION 'reservation identity conflict' USING ERRCODE='23505'; END IF;
 UPDATE finance.dossier x SET version=v+1,status='DRAFT',updated_at=GREATEST(updated_at,clock_timestamp()) WHERE (x.tenant_id,x.company_id,x.id)=(t,c,d);
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES(t,c,cmd,a,cmd,corr,i,'FINANCE_RESERVATION_BOUND');
 RETURN QUERY SELECT i,v+1;
END $$;
ALTER FUNCTION finance.runtime_bind_reservation(uuid,uuid,uuid,uuid,bigint,uuid,uuid[],text,text,uuid,uuid,uuid) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.runtime_bind_reservation(uuid,uuid,uuid,uuid,bigint,uuid,uuid[],text,text,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_bind_reservation(uuid,uuid,uuid,uuid,bigint,uuid,uuid[],text,text,uuid,uuid,uuid) TO finance_runtime;

CREATE OR REPLACE FUNCTION finance.complete_preflight_with_rules(t uuid,c uuid,j uuid,f bigint,w uuid,material jsonb,results jsonb)
RETURNS void LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE current_material jsonb; p finance.preflight_request%ROWTYPE; result jsonb; rule jsonb; evidence uuid[]; seen uuid[]:=ARRAY[]::uuid[]; structural jsonb:='[]'::jsonb; item record; outcome text; detail text; blocked boolean; revision_version bigint; affected bigint; digest bytea;
BEGIN
 current_material:=finance.runtime_preflight_rule_material(t,c,j,f,w);
 IF current_material IS DISTINCT FROM material OR jsonb_typeof(results) IS DISTINCT FROM 'array'
 OR jsonb_array_length(results)<>jsonb_array_length(material->'rules') OR octet_length(results::text)>2097152
 THEN RAISE EXCEPTION 'stale rule evaluation' USING ERRCODE='23514'; END IF;
 SELECT * INTO STRICT p FROM finance.preflight_request WHERE (tenant_id,company_id,job_id)=(t,c,j);
 -- Evaluate current structural evidence, not fixed prototype outcomes.
 -- A clean blob must correspond to the exact finalized generation and digest.
 structural:=structural||jsonb_build_array(jsonb_build_object('key','policy.effective','outcome','PASS','severity','BLOCK','detail','PUBLISHED_POLICY_FOUND'));
 FOR item IN SELECT document_kind,mandatory FROM finance.checklist_item
 WHERE (tenant_id,company_id,policy_id)=(t,c,p.policy_id) ORDER BY document_kind LOOP
  IF EXISTS(SELECT 1 FROM finance.upload_intent u JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(u.tenant_id,u.company_id,u.id)
   WHERE (u.tenant_id,u.company_id,u.dossier_id,u.revision_id,u.actor_id,u.document_kind)=(t,c,p.dossier_id,p.revision_id,p.actor_id,item.document_kind)
   AND u.state='FINALIZED' AND b.generation=u.generation AND b.sha256_digest=u.declared_sha256 AND b.scan_state='CLEAN')
   THEN outcome:='PASS'; detail:='CLEAN_DOCUMENT_FOUND';
  ELSIF EXISTS(SELECT 1 FROM finance.upload_intent u WHERE (u.tenant_id,u.company_id,u.dossier_id,u.revision_id,u.document_kind)=(t,c,p.dossier_id,p.revision_id,item.document_kind) AND u.state<>'CANCELLED')
   THEN outcome:='UNKNOWN'; detail:='DOCUMENT_NOT_RELEASED';
  ELSE outcome:='FAIL'; detail:='MISSING_DOCUMENT'; END IF;
  structural:=structural||jsonb_build_array(jsonb_build_object('key','checklist.'||item.document_kind,'outcome',outcome,'severity',CASE WHEN item.mandatory THEN 'BLOCK' ELSE 'WARNING' END,'detail',detail));
 END LOOP;
 IF EXISTS(SELECT 1 FROM finance.upload_intent u LEFT JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(u.tenant_id,u.company_id,u.id)
 WHERE (u.tenant_id,u.company_id,u.dossier_id,u.revision_id)=(t,c,p.dossier_id,p.revision_id) AND u.state<>'CANCELLED'
 AND (u.state<>'FINALIZED' OR b.id IS NULL OR b.generation<>u.generation OR b.sha256_digest<>u.declared_sha256 OR b.scan_state<>'CLEAN'))
 THEN outcome:='UNKNOWN'; detail:='DOCUMENT_NOT_RELEASED'; ELSE outcome:='PASS'; detail:='CLEAN_GENERATIONS_VERIFIED'; END IF;
 structural:=structural||jsonb_build_array(jsonb_build_object('key','scan.release','outcome',outcome,'severity','BLOCK','detail',detail));
 IF jsonb_array_length(material->'rules')=0 OR EXISTS(SELECT 1 FROM finance.policy_clause pc
 WHERE (pc.tenant_id,pc.company_id,pc.policy_id)=(t,c,p.policy_id)
 AND NOT EXISTS(SELECT 1 FROM finance.checklist_item ci WHERE (ci.tenant_id,ci.company_id,ci.policy_id,ci.clause_id)=(t,c,p.policy_id,pc.id))
 AND NOT EXISTS(SELECT 1 FROM finance.rule_version rv WHERE (rv.tenant_id,rv.company_id,rv.policy_id,rv.clause_id)=(t,c,p.policy_id,pc.id) AND rv.state='PUBLISHED')
 AND NOT EXISTS(SELECT 1 FROM finance.policy_critical_field cf WHERE (cf.tenant_id,cf.company_id,cf.policy_id,cf.clause_id)=(t,c,p.policy_id,pc.id)))
 THEN outcome:='UNKNOWN'; detail:='POLICY_RULE_COVERAGE_INCOMPLETE'; ELSE outcome:='PASS'; detail:='PUBLISHED_RULE_COVERAGE_FOUND'; END IF;
 structural:=structural||jsonb_build_array(jsonb_build_object('key','rules.complete','outcome',outcome,'severity','BLOCK','detail',detail));
 IF NOT EXISTS(SELECT 1 FROM finance.policy_version pv WHERE (pv.tenant_id,pv.company_id,pv.id)=(t,c,p.policy_id) AND pv.criticality_complete)
 THEN outcome:='UNKNOWN'; detail:='NO_APPROVED_CRITICAL_REGISTRY';
 ELSIF EXISTS(SELECT 1 FROM finance.policy_critical_field cf WHERE (cf.tenant_id,cf.company_id,cf.policy_id)=(t,c,p.policy_id)
 AND (SELECT count(*) FROM jsonb_array_elements(material->'fields') fld WHERE fld->>'path'=cf.field_path AND fld->>'verified'='true' AND fld->>'value' IS NOT NULL)<>1)
 THEN outcome:='UNKNOWN'; detail:='CRITICAL_EVIDENCE_INCOMPLETE';
 ELSE outcome:='PASS'; detail:='CRITICAL_EVIDENCE_VERIFIED'; END IF;
 structural:=structural||jsonb_build_array(jsonb_build_object('key','critical.fields','outcome',outcome,'severity','BLOCK','detail',detail));
 IF finance.current_bound_reservation(t,c,p.dossier_id,p.revision_id,p.policy_id,p.dossier_version) IS NULL
 THEN outcome:='UNKNOWN'; detail:='NO_VERIFIED_CURRENT_RESERVATION'; ELSE outcome:='PASS'; detail:='SOURCE_BOUND_RESERVATION_VERIFIED'; END IF;
 structural:=structural||jsonb_build_array(jsonb_build_object('key','financial.reservation','outcome',outcome,'severity','BLOCK','detail',detail));
 -- Validate every worker result before writing any check rows.
 blocked:=EXISTS(SELECT 1 FROM jsonb_array_elements(structural) x WHERE x->>'severity'='BLOCK' AND x->>'outcome' NOT IN ('PASS','NOT_APPLICABLE'))
 OR EXISTS(SELECT 1 FROM jsonb_array_elements(results) x JOIN LATERAL(SELECT r FROM jsonb_array_elements(material->'rules') r WHERE r->>'id'=x->>'id') rr ON true
 WHERE rr.r->>'severity'='BLOCK' AND x->>'outcome' NOT IN ('PASS','NOT_APPLICABLE'));
 revision_version:=p.dossier_version;
 IF NOT blocked THEN revision_version:=revision_version+1; END IF;
 digest:=sha256(convert_to(jsonb_build_array(material,structural,results)::text,'UTF8'));
 INSERT INTO finance.check_run(tenant_id,company_id,id,dossier_id,revision_id,dossier_version,actor_id,policy_id,business_day,input_hash,blocked)
 VALUES(t,c,p.run_id,p.dossier_id,p.revision_id,revision_version,p.actor_id,p.policy_id,p.business_day,digest,blocked);
 FOR item IN SELECT value FROM jsonb_array_elements(structural) LOOP
 INSERT INTO finance.check_result(tenant_id,company_id,run_id,rule_key,outcome,severity,detail_code)
 VALUES(t,c,p.run_id,item.value->>'key',item.value->>'outcome',item.value->>'severity',item.value->>'detail');
 END LOOP;
 FOR result IN SELECT value FROM jsonb_array_elements(results) LOOP
  SELECT value INTO rule FROM jsonb_array_elements(material->'rules') WHERE value->>'id'=result->>'id';
  IF NOT FOUND OR (result->>'version')::bigint IS DISTINCT FROM (rule->>'version')::bigint
  OR (result->>'id')::uuid=ANY(seen) OR result->>'outcome' IS NULL OR result->>'outcome' NOT IN ('PASS','FAIL','UNKNOWN','ERROR','NOT_APPLICABLE')
  OR result->>'detail' IS NULL OR result->>'detail' NOT IN ('RULE_EVALUATED','RULE_COMPILE_ERROR')
  OR jsonb_typeof(result->'evidence') IS DISTINCT FROM 'array' OR jsonb_array_length(result->'evidence')>256
  THEN RAISE EXCEPTION 'invalid rule result' USING ERRCODE='23514'; END IF;
  seen:=array_append(seen,(result->>'id')::uuid);
  SELECT coalesce(array_agg(DISTINCT x::uuid ORDER BY x::uuid),ARRAY[]::uuid[]) INTO evidence FROM jsonb_array_elements_text(result->'evidence') x;
  IF cardinality(evidence)<>jsonb_array_length(result->'evidence') OR EXISTS(
   SELECT 1 FROM unnest(evidence) e WHERE NOT EXISTS(
    SELECT 1 FROM jsonb_array_elements(material->'fields') fld
    CROSS JOIN LATERAL jsonb_array_elements_text(fld->'evidence') ev
    WHERE fld->>'verified'='true' AND ev=e::text))
  THEN RAISE EXCEPTION 'unbound rule evidence' USING ERRCODE='23514'; END IF;
  INSERT INTO finance.check_result(tenant_id,company_id,run_id,rule_key,outcome,severity,detail_code,rule_version,evidence_ids)
  VALUES(t,c,p.run_id,'dsl.'||(rule->>'name'),result->>'outcome',rule->>'severity',result->>'detail',(rule->>'version')::bigint,evidence);
 END LOOP;
 IF NOT blocked THEN
  UPDATE finance.dossier SET status='REVIEWABLE',version=revision_version,updated_at=GREATEST(updated_at,clock_timestamp())
  WHERE (tenant_id,company_id,id,version)=(t,c,p.dossier_id,p.dossier_version) AND status='DRAFT';
  IF NOT FOUND THEN RAISE EXCEPTION 'stale preflight dossier' USING ERRCODE='23514'; END IF;
 END IF;
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
 VALUES(t,c,p.run_id,p.actor_id,j,j,p.dossier_id,'SYNTHETIC_PREFLIGHT_CHECKED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
 VALUES(t,c,p.run_id,j,p.dossier_id,'SYNTHETIC_PREFLIGHT_CHECKED');
 UPDATE finance.job SET state='SUCCEEDED',lease_until=NULL,leased_by=NULL
 WHERE (tenant_id,company_id,id,fence,leased_by)=(t,c,j,f,w) AND state='LEASED' AND lease_until>clock_timestamp();
 IF NOT FOUND THEN RAISE EXCEPTION 'stale preflight lease' USING ERRCODE='40001'; END IF;
 UPDATE finance.job_attempt SET state='SUCCEEDED',finished_at=clock_timestamp()
 WHERE (tenant_id,company_id,job_id,fence,worker_id)=(t,c,j,f,w) AND state='ACTIVE';
 IF NOT FOUND THEN RAISE EXCEPTION 'stale attempt' USING ERRCODE='40001'; END IF;
 INSERT INTO finance.inbox(tenant_id,company_id,event_id,fence) VALUES(t,c,j,f);
END $$;
ALTER FUNCTION finance.complete_preflight_with_rules(uuid,uuid,uuid,bigint,uuid,jsonb,jsonb) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.complete_preflight_with_rules(uuid,uuid,uuid,bigint,uuid,jsonb,jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.complete_preflight_with_rules(uuid,uuid,uuid,bigint,uuid,jsonb,jsonb) TO finance_job_worker;

CREATE OR REPLACE FUNCTION finance.workflow_ready(
 t uuid,c uuid,d uuid,v bigint,a uuid,check_id uuid,snap_id uuid,cmd uuid,corr uuid
) RETURNS uuid
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; cr finance.check_run%ROWTYPE;
 policy_row finance.policy_version%ROWTYPE; form_row finance.form_version%ROWTYPE;
 template_row finance.template_version%ROWTYPE; reservation_id uuid; manifest bytea; ack_manifest jsonb; render_material jsonb;
BEGIN
 IF t IS NULL OR c IS NULL OR d IS NULL OR a IS NULL OR check_id IS NULL OR snap_id IS NULL
 OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR a IS DISTINCT FROM finance.scope_actor()
 THEN RAISE EXCEPTION 'invalid ready command' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id<>a OR ds.status<>'REVIEWABLE' OR ds.version<>v
 OR NOT EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c
  AND m.actor_id=a AND m.role='REQUESTER' AND m.active
  AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'ready authority or state' USING ERRCODE='23514'; END IF;
 SELECT id INTO STRICT rev FROM finance.dossier_revision
  WHERE tenant_id=t AND company_id=c AND dossier_id=d AND number=ds.revision;
 SELECT * INTO cr FROM finance.check_run
  WHERE tenant_id=t AND company_id=c AND id=check_id AND dossier_id=d
  AND revision_id=rev AND dossier_version=v AND actor_id=a;
 IF NOT FOUND OR cr.blocked OR cr.policy_id IS NULL
 OR EXISTS(SELECT 1 FROM finance.check_run newer WHERE newer.tenant_id=t AND newer.company_id=c
   AND newer.dossier_id=d AND newer.revision_id=rev AND newer.dossier_version=v
   AND (newer.created_at,newer.id)>(cr.created_at,cr.id))
 OR NOT EXISTS(SELECT 1 FROM finance.check_result x WHERE x.tenant_id=t AND x.company_id=c
   AND x.run_id=check_id)
 OR EXISTS(SELECT 1 FROM finance.check_result x WHERE x.tenant_id=t AND x.company_id=c
   AND x.run_id=check_id AND x.severity='BLOCK' AND x.outcome NOT IN ('PASS','NOT_APPLICABLE'))
 THEN RAISE EXCEPTION 'blocking checks' USING ERRCODE='23514'; END IF;
 IF EXISTS(SELECT 1 FROM finance.check_result x
  WHERE x.tenant_id=t AND x.company_id=c AND x.run_id=check_id
   AND x.severity='WARNING' AND x.outcome='FAIL' AND NOT EXISTS(
    SELECT 1 FROM finance.check_acknowledgment w
    WHERE (w.tenant_id,w.company_id,w.run_id,w.rule_key,w.actor_id)=(t,c,check_id,x.rule_key,a)))
 THEN RAISE EXCEPTION 'warning acknowledgment required' USING ERRCODE='23514'; END IF;
 SELECT COALESCE(jsonb_agg(jsonb_build_array(w.id,w.rule_key,w.actor_id,w.reason)
    ORDER BY w.rule_key,w.id),'[]'::jsonb) INTO ack_manifest
 FROM finance.check_acknowledgment w
 JOIN finance.check_result x ON (x.tenant_id,x.company_id,x.run_id,x.rule_key)=
  (w.tenant_id,w.company_id,w.run_id,w.rule_key)
 WHERE (w.tenant_id,w.company_id,w.run_id,w.actor_id)=(t,c,check_id,a)
  AND x.severity='WARNING' AND x.outcome='FAIL';
 PERFORM finance.prototype_policy_lock(t,c,ds.kind);
 SELECT * INTO policy_row FROM finance.policy_version
  WHERE tenant_id=t AND company_id=c AND id=cr.policy_id AND state='PUBLISHED'
  AND dossier_kind=ds.kind AND effective_from<=cr.business_day AND cr.business_day<effective_until;
 IF NOT FOUND THEN RAISE EXCEPTION 'effective policy unavailable' USING ERRCODE='23514'; END IF;
 IF EXISTS(SELECT 1 FROM finance.checklist_item ci WHERE ci.tenant_id=t AND ci.company_id=c
   AND ci.policy_id=cr.policy_id AND ci.mandatory AND NOT EXISTS(
    SELECT 1 FROM finance.upload_intent u JOIN finance.document_blob b
     ON (b.tenant_id,b.company_id,b.id)=(u.tenant_id,u.company_id,u.id)
    WHERE u.tenant_id=t AND u.company_id=c AND u.dossier_id=d AND u.revision_id=rev
     AND u.actor_id=a AND u.document_kind=ci.document_kind AND u.state='FINALIZED' AND b.scan_state='CLEAN'))
 THEN RAISE EXCEPTION 'mandatory document missing' USING ERRCODE='23514'; END IF;
 SELECT * INTO form_row FROM finance.form_version WHERE tenant_id=t AND company_id=c
  AND policy_id=cr.policy_id AND state='PUBLISHED' ORDER BY version DESC,id DESC LIMIT 1;
 IF NOT FOUND THEN RAISE EXCEPTION 'published form unavailable' USING ERRCODE='23514'; END IF;
 SELECT * INTO template_row FROM finance.template_version WHERE tenant_id=t AND company_id=c
  AND policy_id=cr.policy_id AND state='PUBLISHED' ORDER BY version DESC,id DESC LIMIT 1;
 IF NOT FOUND THEN RAISE EXCEPTION 'published template unavailable' USING ERRCODE='23514'; END IF;
 reservation_id:=finance.current_bound_reservation(t,c,d,rev,cr.policy_id,v);
 IF reservation_id IS NULL THEN RAISE EXCEPTION 'active reservation required' USING ERRCODE='23514'; END IF;
 IF cr.business_day IS DISTINCT FROM (clock_timestamp() AT TIME ZONE 'UTC')::date OR cr.financial_digest IS DISTINCT FROM sha256(convert_to(finance.runtime_verified_financial_facts(t,c,d,rev,cr.policy_id,v)::text,'UTF8')) THEN RAISE EXCEPTION 'financial facts changed; rerun preflight' USING ERRCODE='23514'; END IF;
 IF EXISTS(SELECT 1 FROM finance.upload_intent u WHERE u.tenant_id=t AND u.company_id=c
   AND u.dossier_id=d AND u.revision_id=rev AND u.state<>'CANCELLED'
   AND (u.state<>'FINALIZED' OR NOT EXISTS(SELECT 1 FROM finance.document_blob b
     WHERE b.tenant_id=u.tenant_id AND b.company_id=u.company_id AND b.id=u.id
     AND b.scan_state='CLEAN')))
 THEN RAISE EXCEPTION 'unclean document' USING ERRCODE='23514'; END IF;
 render_material:=jsonb_build_object(
 'schema','finance.export.material.v1','dossier',d,'revision',rev,'snapshot',snap_id,
 'template',template_row.id,'template_digest',encode(template_row.source_digest,'hex'),
 'fields',COALESCE((SELECT jsonb_agg(row.item ORDER BY row.path,row.document) FROM (
 SELECT f.field_path AS path,o.document_id AS document,jsonb_build_object('path',f.field_path,'document',o.document_id,'value',q.normalized_value,'evidence',q.evidence_ids) AS item
 FROM finance.document_ref dr JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(dr.tenant_id,dr.company_id,dr.id)
 JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id)=(dr.tenant_id,dr.company_id,dr.id)
 JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.document_id,o.revision_id,o.generation)=(dr.tenant_id,dr.company_id,dr.id,dr.revision_id,b.generation)
 JOIN finance.extraction_run x ON (x.tenant_id,x.company_id,x.ocr_run_id)=(o.tenant_id,o.company_id,o.id)
 JOIN finance.extracted_field f ON (f.tenant_id,f.company_id,f.run_id)=(x.tenant_id,x.company_id,x.id)
 JOIN LATERAL (SELECT q.disposition,q.normalized_value,q.evidence_ids FROM finance.field_decision q
 WHERE (q.tenant_id,q.company_id,q.run_id,q.field_path,q.dossier_id,q.revision_id)=(f.tenant_id,f.company_id,f.run_id,f.field_path,d,rev)
 AND q.dossier_version<=v ORDER BY q.dossier_version DESC,q.created_at DESC,q.id DESC LIMIT 1) q ON true
 WHERE (dr.tenant_id,dr.company_id,dr.dossier_id,dr.revision_id)=(t,c,d,rev)
 AND b.scan_state='CLEAN' AND u.state='FINALIZED' AND u.generation=b.generation AND o.source_digest=b.sha256_digest
 AND q.disposition IN ('CONFIRMED_CANDIDATE','CORRECTED') AND cardinality(q.evidence_ids)>0
 AND NOT EXISTS(SELECT 1 FROM finance.ocr_run newer WHERE (newer.tenant_id,newer.company_id,newer.document_id,newer.revision_id)=(o.tenant_id,o.company_id,o.document_id,o.revision_id)
 AND (newer.created_at,newer.id)>(o.created_at,o.id))
 LIMIT 5001) row),'[]'::jsonb),
 'documents',COALESCE((SELECT jsonb_agg(jsonb_build_object('id',b.id,'sha256',encode(b.sha256_digest,'hex'),'mime',b.mime,'bytes',b.byte_count,'kind',u.document_kind) ORDER BY b.id)
 FROM finance.upload_intent u JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(u.tenant_id,u.company_id,u.id)
 WHERE (u.tenant_id,u.company_id,u.dossier_id,u.revision_id)=(t,c,d,rev) AND u.state='FINALIZED' AND u.generation=b.generation AND b.scan_state='CLEAN'),'[]'::jsonb),
 'checks',COALESCE((SELECT jsonb_agg(jsonb_build_object('key',x.rule_key,'outcome',x.outcome,'severity',x.severity,'detail',x.detail_code,'version',x.rule_version,'evidence',x.evidence_ids) ORDER BY x.rule_key)
 FROM finance.check_result x WHERE (x.tenant_id,x.company_id,x.run_id)=(t,c,check_id)),'[]'::jsonb),
 'form',form_row.schema_json);
 IF jsonb_array_length(render_material->'fields')>5000 OR octet_length(render_material::text)>16777216 THEN RAISE EXCEPTION 'export material too large' USING ERRCODE='23514'; END IF;
 manifest:=sha256(convert_to(jsonb_build_array(
  d,rev,v,check_id,encode(cr.input_hash,'hex'),cr.policy_id,
  form_row.id,template_row.id,reservation_id,(SELECT encode(x.binding_material_hash,'hex') FROM finance.reservation x WHERE (x.tenant_id,x.company_id,x.id)=(t,c,reservation_id)),ack_manifest,render_material
 )::text,'UTF8'));
 INSERT INTO finance.dossier_snapshot(
  tenant_id,company_id,id,dossier_id,revision_id,policy_id,check_run_id,input_hash,
  canonical_manifest_hash,form_version_id,template_version_id,reservation_id,export_material
 ) VALUES(t,c,snap_id,d,rev,cr.policy_id,check_id,cr.input_hash,manifest,
  form_row.id,template_row.id,reservation_id,render_material);
 PERFORM finance.seal_acceptance_claims(t,c,d,rev,cr.policy_id,reservation_id,snap_id);
 PERFORM finance.seal_extra_settlement(t,c,d,rev,cr.policy_id,reservation_id,v,snap_id);
 INSERT INTO finance.snapshot_member(tenant_id,company_id,snapshot_id,id,kind,resource_id,digest) VALUES
  (t,c,snap_id,gen_random_uuid(),'CHECK_RUN',check_id,cr.input_hash),
  (t,c,snap_id,gen_random_uuid(),'POLICY',policy_row.id,policy_row.source_digest),
  (t,c,snap_id,gen_random_uuid(),'FORM',form_row.id,sha256(convert_to(form_row.schema_json::text,'UTF8'))),
  (t,c,snap_id,gen_random_uuid(),'TEMPLATE',template_row.id,template_row.source_digest),
  (t,c,snap_id,gen_random_uuid(),'RESERVATION',reservation_id,
   sha256(convert_to(jsonb_build_array(reservation_id)::text,'UTF8')));
 INSERT INTO finance.snapshot_member(tenant_id,company_id,snapshot_id,id,kind,resource_id,digest)
 SELECT t,c,snap_id,gen_random_uuid(),'WARNING_ACK',w.id,
  sha256(convert_to(jsonb_build_array(w.id,w.rule_key,w.actor_id,w.reason)::text,'UTF8'))
 FROM finance.check_acknowledgment w JOIN finance.check_result x
  ON (x.tenant_id,x.company_id,x.run_id,x.rule_key)=(w.tenant_id,w.company_id,w.run_id,w.rule_key)
 WHERE (w.tenant_id,w.company_id,w.run_id,w.actor_id)=(t,c,check_id,a)
  AND x.severity='WARNING' AND x.outcome='FAIL';
 UPDATE finance.dossier SET status='READY',version=version+1,updated_at=GREATEST(updated_at,clock_timestamp())
  WHERE tenant_id=t AND company_id=c AND id=d;
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,d,'DOSSIER_READY');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,cmd,cmd,d,'DOSSIER_READY');
 RETURN snap_id;
END $$;

