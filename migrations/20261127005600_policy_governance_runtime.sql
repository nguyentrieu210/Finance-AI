-- E16: authenticated policy governance over source-bound drafts. No implicit source or publication.
ALTER TABLE finance.policy_version ADD COLUMN aggregate_version bigint NOT NULL DEFAULT 1 CHECK(aggregate_version BETWEEN 1 AND 9007199254740991),
 ADD COLUMN display_name text CHECK(display_name IS NULL OR (octet_length(display_name) BETWEEN 1 AND 200 AND btrim(display_name)<>'')),
 ADD COLUMN supersedes_id uuid,
 ADD FOREIGN KEY(tenant_id,company_id,supersedes_id) REFERENCES finance.policy_version(tenant_id,company_id,id);
ALTER TABLE finance.idempotency_record DROP CONSTRAINT dossier_change_receipt_required,
 ADD CONSTRAINT dossier_change_receipt_required CHECK(
 CASE WHEN operation_id IN ('dossier.revise','dossier.reopen','dossier.cancel','policy.submit','policy.publish','policy.retire','policy.revoke','policy.revise')
 THEN result_payload IS NOT NULL AND jsonb_typeof(result_payload)='object'
 ELSE result_payload IS NULL END);
CREATE FUNCTION finance.policy_version_state_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'immutable policy version' USING ERRCODE='23514'; END IF;
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.dossier_kind,NEW.author_id,NEW.effective_from,NEW.effective_until,
     NEW.source_digest,NEW.display_name,NEW.supersedes_id,NEW.created_at)
 IS DISTINCT FROM (OLD.tenant_id,OLD.company_id,OLD.id,OLD.dossier_kind,OLD.author_id,OLD.effective_from,OLD.effective_until,
     OLD.source_digest,OLD.display_name,OLD.supersedes_id,OLD.created_at)
 THEN RAISE EXCEPTION 'immutable policy content' USING ERRCODE='23514'; END IF;
 IF (OLD.state,NEW.state) NOT IN (('DRAFT','IN_REVIEW'),('IN_REVIEW','PUBLISHED'),('PUBLISHED','RETIRED'),('PUBLISHED','REVOKED'))
 THEN RAISE EXCEPTION 'invalid policy transition' USING ERRCODE='23514'; END IF;
 IF OLD.state='PUBLISHED' AND (NEW.checker_id,NEW.published_at) IS DISTINCT FROM (OLD.checker_id,OLD.published_at)
 THEN RAISE EXCEPTION 'immutable approval' USING ERRCODE='23514'; END IF;
 NEW.aggregate_version:=OLD.aggregate_version+1;
 RETURN NEW;
END $$;
CREATE TRIGGER policy_version_state_guard BEFORE UPDATE OR DELETE ON finance.policy_version
 FOR EACH ROW EXECUTE FUNCTION finance.policy_version_state_guard();

CREATE ROLE finance_policy_governance NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_policy_governance;
GRANT SELECT ON finance.policy_version,finance.policy_clause,finance.checklist_item,finance.membership,
 finance.rule_version,finance.form_version,finance.template_version,finance.workflow_version,finance.idempotency_record TO finance_policy_governance;
GRANT INSERT ON finance.policy_version,finance.policy_clause,finance.checklist_item,finance.audit_event,finance.outbox,finance.idempotency_record TO finance_policy_governance;
GRANT UPDATE(state,checker_id,published_at) ON finance.policy_version TO finance_policy_governance;
GRANT EXECUTE ON FUNCTION finance.prototype_policy_lock(uuid,uuid,text),finance.prototype_policy_transition(uuid,uuid,uuid,uuid,text) TO finance_policy_governance;
CREATE POLICY policy_governance_read ON finance.policy_version FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY policy_governance_insert ON finance.policy_version FOR INSERT TO finance_policy_governance
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND author_id=finance.scope_actor() AND state='DRAFT');
CREATE POLICY policy_governance_update ON finance.policy_version FOR UPDATE TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY governance_policy_clause_read ON finance.policy_clause FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY governance_checklist_item_read ON finance.checklist_item FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY governance_membership_read ON finance.membership FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY governance_rule_version_read ON finance.rule_version FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY governance_form_version_read ON finance.form_version FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY governance_template_version_read ON finance.template_version FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY governance_workflow_version_read ON finance.workflow_version FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY governance_policy_clause_insert ON finance.policy_clause FOR INSERT TO finance_policy_governance
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND EXISTS(
 SELECT 1 FROM finance.policy_version p WHERE (p.tenant_id,p.company_id,p.id)=(policy_clause.tenant_id,policy_clause.company_id,policy_clause.policy_id)
 AND p.author_id=finance.scope_actor() AND p.state='DRAFT'));
CREATE POLICY governance_checklist_item_insert ON finance.checklist_item FOR INSERT TO finance_policy_governance
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND EXISTS(
 SELECT 1 FROM finance.policy_version p WHERE (p.tenant_id,p.company_id,p.id)=(checklist_item.tenant_id,checklist_item.company_id,checklist_item.policy_id)
 AND p.author_id=finance.scope_actor() AND p.state='DRAFT'));
CREATE POLICY governance_idem_read ON finance.idempotency_record FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY governance_idem_insert ON finance.idempotency_record FOR INSERT TO finance_policy_governance
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor() AND operation_id LIKE 'policy.%');
CREATE POLICY governance_audit_insert ON finance.audit_event FOR INSERT TO finance_policy_governance
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor() AND action LIKE 'POLICY_%');
CREATE POLICY governance_outbox_insert ON finance.outbox FOR INSERT TO finance_policy_governance
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND event_type LIKE 'POLICY_%');
CREATE POLICY policy_maker_history ON finance.policy_version FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND author_id=finance.scope_actor()
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(finance.scope_tenant(),finance.scope_company(),finance.scope_actor())
 AND m.role='POLICY_ADMIN' AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())));
CREATE POLICY policy_checker_read ON finance.policy_version FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND state<>'DRAFT'
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(finance.scope_tenant(),finance.scope_company(),finance.scope_actor())
 AND m.role='FINANCE_APPROVER' AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())));

CREATE FUNCTION finance.runtime_policy_change(t uuid,c uuid,a uuid,pid uuid,v bigint,op text,idem text,why text,new_name text,day date,new_id uuid,cmd uuid,corr uuid)
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
   PERFORM finance.prototype_policy_transition(t,c,pid,a,'PUBLISHED'); action_name:='POLICY_PUBLISHED';
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
ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','rule.revise','warning.ack','dossier.revise','dossier.reopen','dossier.cancel','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide','match.confirm','review.begin','review.decide','membership.create','membership.revoke',
 'risk.resolve','policy.submit','policy.publish','policy.retire','policy.revoke','policy.revise','workflow.create','workflow.revise'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','WARNING_ACKNOWLEDGED','DOSSIER_REVISED','DOSSIER_REOPENED','DOSSIER_CANCELLED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','POLICY_SUBMITTED','POLICY_PUBLISHED','POLICY_RETIRED','POLICY_REVOKED','POLICY_REVISED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','WARNING_ACKNOWLEDGED','DOSSIER_REVISED','DOSSIER_REOPENED','DOSSIER_CANCELLED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','POLICY_SUBMITTED','POLICY_PUBLISHED','POLICY_RETIRED','POLICY_REVOKED','POLICY_REVISED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
