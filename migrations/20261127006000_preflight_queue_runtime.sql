-- E15: queue and execute the conservative preflight against a pinned dossier/policy.
CREATE TABLE finance.preflight_request (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,job_id uuid NOT NULL,run_id uuid NOT NULL,
 dossier_id uuid NOT NULL,revision_id uuid NOT NULL,actor_id uuid NOT NULL,
 dossier_version bigint NOT NULL CHECK(dossier_version>=0),policy_id uuid NOT NULL,business_day date NOT NULL,
 PRIMARY KEY(tenant_id,company_id,job_id),UNIQUE(tenant_id,company_id,run_id),
 FOREIGN KEY(tenant_id,company_id,job_id) REFERENCES finance.outbox(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,dossier_id) REFERENCES finance.dossier(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,revision_id) REFERENCES finance.dossier_revision(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,policy_id) REFERENCES finance.policy_version(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id)
);
ALTER TABLE finance.preflight_request ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.preflight_request FORCE ROW LEVEL SECURITY;
CREATE TRIGGER preflight_request_immutable BEFORE UPDATE OR DELETE ON finance.preflight_request
 FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
GRANT SELECT,INSERT ON finance.preflight_request TO finance_workflow_writer;
CREATE POLICY preflight_request_read ON finance.preflight_request FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY preflight_request_insert ON finance.preflight_request FOR INSERT TO finance_workflow_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
GRANT SELECT,INSERT ON finance.idempotency_record TO finance_workflow_writer;
CREATE POLICY preflight_idem_read ON finance.idempotency_record FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor() AND operation_id='checks.run');
CREATE POLICY preflight_idem_insert ON finance.idempotency_record FOR INSERT TO finance_workflow_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor() AND operation_id='checks.run');
GRANT EXECUTE ON FUNCTION finance.prototype_policy_lock(uuid,uuid,text),finance.prototype_effective_policy(uuid,uuid,text,date),
 finance.prototype_preflight(uuid,uuid,uuid,bigint,uuid,uuid,date) TO finance_workflow_writer;
GRANT SELECT ON finance.field_decision,finance.ocr_run,finance.document_ref,finance.checklist_item TO finance_workflow_writer;
CREATE POLICY preflight_field_decision_read ON finance.field_decision FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY preflight_ocr_run_read ON finance.ocr_run FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY preflight_document_ref_read ON finance.document_ref FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY preflight_checklist_item_read ON finance.checklist_item FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
GRANT INSERT ON finance.check_run,finance.check_result TO finance_workflow_writer;
CREATE POLICY preflight_run_insert ON finance.check_run FOR INSERT TO finance_workflow_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY preflight_result_insert ON finance.check_result FOR INSERT TO finance_workflow_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND EXISTS(
  SELECT 1 FROM finance.check_run r WHERE (r.tenant_id,r.company_id,r.id,r.actor_id)=(check_result.tenant_id,check_result.company_id,check_result.run_id,finance.scope_actor())));

CREATE FUNCTION finance.runtime_request_preflight(t uuid,c uuid,a uuid,d uuid,v bigint,p uuid,r uuid,j uuid,corr uuid,idem text)
RETURNS uuid LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; day date; digest bytea; previous bytea; receipt uuid;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR v IS NULL OR p IS NULL OR r IS NULL OR j IS NULL OR corr IS NULL
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
 OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid preflight request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
 AND m.role='REQUESTER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'requester required' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.preflight/v1/'||t::text||'/'||c::text||'/'||a::text||'/'||idem,0));
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id<>a THEN RAISE EXCEPTION 'dossier authority' USING ERRCODE='42501'; END IF;
 digest:=sha256(convert_to(jsonb_build_array(d,v,p)::text,'UTF8'));
 SELECT ir.request_hash,ir.result_id INTO previous,receipt FROM finance.idempotency_record ir
 WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'checks.run',idem);
 IF FOUND THEN
  IF previous<>digest THEN RAISE EXCEPTION 'preflight replay conflict' USING ERRCODE='23514'; END IF;
  RETURN receipt;
 END IF;
 IF ds.version<>v OR ds.status<>'DRAFT' THEN RAISE EXCEPTION 'stale preflight' USING ERRCODE='23514'; END IF;
 SELECT id INTO STRICT rev FROM finance.dossier_revision WHERE tenant_id=t AND company_id=c AND dossier_id=d AND number=ds.revision;
 day:=(clock_timestamp() AT TIME ZONE 'UTC')::date;
 PERFORM finance.prototype_policy_lock(t,c,ds.kind);
 IF p IS DISTINCT FROM finance.prototype_effective_policy(t,c,ds.kind,day)
 THEN RAISE EXCEPTION 'effective policy mismatch' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
 VALUES(t,c,j,a,j,corr,d,'PREFLIGHT_REQUESTED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
 VALUES(t,c,j,j,d,'PREFLIGHT_REQUESTED');
 INSERT INTO finance.preflight_request(tenant_id,company_id,job_id,run_id,dossier_id,revision_id,actor_id,dossier_version,policy_id,business_day)
 VALUES(t,c,j,r,d,rev,a,v,p,day);
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_version)
 VALUES(t,c,a,'checks.run',idem,digest,j,v);
 RETURN j;
END $$;
ALTER FUNCTION finance.runtime_request_preflight(uuid,uuid,uuid,uuid,bigint,uuid,uuid,uuid,uuid,text) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.runtime_request_preflight(uuid,uuid,uuid,uuid,bigint,uuid,uuid,uuid,uuid,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_request_preflight(uuid,uuid,uuid,uuid,bigint,uuid,uuid,uuid,uuid,text) TO finance_runtime;

GRANT SELECT ON finance.worker_scope,finance.job,finance.job_attempt TO finance_workflow_writer;
GRANT EXECUTE ON FUNCTION finance.worker_scope_allowed(uuid,uuid) TO finance_workflow_writer;
GRANT UPDATE(state,lease_until,leased_by) ON finance.job TO finance_workflow_writer;
GRANT UPDATE(state,finished_at) ON finance.job_attempt TO finance_workflow_writer;
GRANT INSERT ON finance.inbox TO finance_workflow_writer;
CREATE POLICY preflight_worker_scope ON finance.worker_scope FOR SELECT TO finance_workflow_writer USING(login_name=session_user AND active);
CREATE POLICY preflight_job_scope ON finance.job TO finance_workflow_writer
 USING(finance.worker_scope_allowed(tenant_id,company_id)) WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
CREATE POLICY preflight_attempt_scope ON finance.job_attempt TO finance_workflow_writer
 USING(finance.worker_scope_allowed(tenant_id,company_id)) WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
CREATE POLICY preflight_inbox_insert ON finance.inbox FOR INSERT TO finance_workflow_writer
 WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
CREATE FUNCTION finance.complete_preflight_job(t uuid,c uuid,j uuid,f bigint,w uuid)
RETURNS void LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE p finance.preflight_request%ROWTYPE; ds finance.dossier%ROWTYPE; affected bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR j IS NULL OR w IS NULL OR f IS NULL OR f<1
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR NOT finance.worker_scope_allowed(t,c)
 THEN RAISE EXCEPTION 'worker authority' USING ERRCODE='42501'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.job WHERE tenant_id=t AND company_id=c AND id=j
  AND state='LEASED' AND fence=f AND leased_by=w AND lease_until>clock_timestamp() FOR UPDATE)
 THEN RAISE EXCEPTION 'stale preflight lease' USING ERRCODE='40001'; END IF;
 SELECT * INTO p FROM finance.preflight_request WHERE tenant_id=t AND company_id=c AND job_id=j;
 IF NOT FOUND THEN RAISE EXCEPTION 'not a preflight job' USING ERRCODE='P0002'; END IF;
 PERFORM set_config('finance.actor_id',p.actor_id::text,true);
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=p.dossier_id FOR UPDATE;
 IF NOT FOUND OR ds.owner_id<>p.actor_id OR ds.version<>p.dossier_version OR ds.status<>'DRAFT'
 OR NOT EXISTS(SELECT 1 FROM finance.dossier_revision dr WHERE (dr.tenant_id,dr.company_id,dr.dossier_id,dr.id,dr.number)=(t,c,ds.id,p.revision_id,ds.revision))
 OR p.business_day<>(clock_timestamp() AT TIME ZONE 'UTC')::date
 THEN RAISE EXCEPTION 'stale preflight input' USING ERRCODE='23514'; END IF;
 PERFORM finance.prototype_policy_lock(t,c,ds.kind);
 IF p.policy_id IS DISTINCT FROM finance.prototype_effective_policy(t,c,ds.kind,p.business_day)
 THEN RAISE EXCEPTION 'stale policy' USING ERRCODE='23514'; END IF;
 PERFORM finance.prototype_preflight(t,c,ds.id,ds.version,p.actor_id,p.run_id,p.business_day);
 UPDATE finance.job SET state='SUCCEEDED',lease_until=NULL,leased_by=NULL
 WHERE tenant_id=t AND company_id=c AND id=j AND fence=f AND leased_by=w AND state='LEASED' AND lease_until>clock_timestamp();
 GET DIAGNOSTICS affected=ROW_COUNT;
 IF affected<>1 THEN RAISE EXCEPTION 'stale preflight lease' USING ERRCODE='40001'; END IF;
 UPDATE finance.job_attempt SET state='SUCCEEDED',finished_at=clock_timestamp()
 WHERE tenant_id=t AND company_id=c AND job_id=j AND fence=f AND state='ACTIVE';
 GET DIAGNOSTICS affected=ROW_COUNT;
 IF affected<>1 THEN RAISE EXCEPTION 'invalid attempt' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.inbox(tenant_id,company_id,event_id,fence) VALUES(t,c,j,f);
END $$;
ALTER FUNCTION finance.complete_preflight_job(uuid,uuid,uuid,bigint,uuid) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.complete_preflight_job(uuid,uuid,uuid,bigint,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.complete_preflight_job(uuid,uuid,uuid,bigint,uuid) TO finance_job_worker;
ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','rule.revise','rule.simulate','checks.run','warning.ack','dossier.revise','dossier.reopen','dossier.cancel','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide','match.confirm','review.begin','review.decide','membership.create','membership.revoke',
 'risk.resolve','policy.create','policy.submit','policy.publish','policy.retire','policy.revoke','policy.revise','workflow.create','workflow.revise'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','DOCUMENT_DOWNLOADED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','PREFLIGHT_REQUESTED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','RULE_SIMULATED','WARNING_ACKNOWLEDGED','DOSSIER_REVISED','DOSSIER_REOPENED','DOSSIER_CANCELLED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','POLICY_CREATED','POLICY_SUBMITTED','POLICY_PUBLISHED','POLICY_RETIRED','POLICY_REVOKED','POLICY_REVISED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','PREFLIGHT_REQUESTED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','RULE_SIMULATED','WARNING_ACKNOWLEDGED','DOSSIER_REVISED','DOSSIER_REOPENED','DOSSIER_CANCELLED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','POLICY_CREATED','POLICY_SUBMITTED','POLICY_PUBLISHED','POLICY_RETIRED','POLICY_REVOKED','POLICY_REVISED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
