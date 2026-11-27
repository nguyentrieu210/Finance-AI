-- E18: acknowledge an immutable warning without converting its outcome into PASS.
GRANT SELECT,INSERT ON finance.check_acknowledgment TO finance_workflow_writer;
CREATE POLICY workflow_ack_read ON finance.check_acknowledgment FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY workflow_ack_insert ON finance.check_acknowledgment FOR INSERT TO finance_workflow_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE FUNCTION finance.workflow_acknowledge_warning(t uuid,c uuid,a uuid,d uuid,v bigint,result uuid,ack uuid,why text,cmd uuid,corr uuid)
RETURNS void LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; run uuid; rk text;
BEGIN
 IF t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR a IS DISTINCT FROM finance.scope_actor() OR t IS NULL OR c IS NULL OR a IS NULL
 OR d IS NULL OR v IS NULL OR result IS NULL OR ack IS NULL OR cmd IS NULL OR corr IS NULL
 OR why IS NULL OR octet_length(why) NOT BETWEEN 1 AND 2048 OR btrim(why)='' OR why ~ '[[:cntrl:]]'
 THEN RAISE EXCEPTION 'invalid acknowledgment' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id<>a OR ds.version<>v OR ds.status NOT IN ('DRAFT','NEEDS_INPUT','REVIEWABLE')
 OR NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='REQUESTER' AND m.active AND m.valid_from<=clock_timestamp()
  AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'warning authority or state' USING ERRCODE='23514'; END IF;
 SELECT r.id INTO run FROM finance.check_run r JOIN finance.dossier_revision dr
  ON (dr.tenant_id,dr.company_id,dr.id,dr.dossier_id,dr.number)=(r.tenant_id,r.company_id,r.revision_id,d,ds.revision)
 WHERE (r.tenant_id,r.company_id,r.dossier_id,r.actor_id,r.dossier_version)=(t,c,d,a,v)
 ORDER BY r.created_at DESC,r.id DESC LIMIT 1;
 SELECT x.rule_key INTO rk FROM finance.check_result x WHERE (x.tenant_id,x.company_id,x.run_id,x.id)=(t,c,run,result)
  AND x.severity='WARNING' AND x.outcome='FAIL';
 IF NOT FOUND THEN RAISE EXCEPTION 'not a current failed warning' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.check_acknowledgment(tenant_id,company_id,id,run_id,rule_key,actor_id,reason)
 VALUES(t,c,ack,run,rk,a,why);
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
 VALUES(t,c,cmd,a,cmd,corr,ack,'WARNING_ACKNOWLEDGED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
 VALUES(t,c,cmd,cmd,ack,'WARNING_ACKNOWLEDGED');
END $$;
ALTER FUNCTION finance.workflow_acknowledge_warning(uuid,uuid,uuid,uuid,bigint,uuid,uuid,text,uuid,uuid) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.workflow_acknowledge_warning(uuid,uuid,uuid,uuid,bigint,uuid,uuid,text,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.workflow_acknowledge_warning(uuid,uuid,uuid,uuid,bigint,uuid,uuid,text,uuid,uuid) TO finance_runtime;

CREATE FUNCTION finance.runtime_warning_receipt(t uuid,c uuid,a uuid,d uuid,ack uuid,v bigint)
RETURNS TABLE(id uuid,rule_id text,rule_version bigint,outcome text,severity text,reason_code text,evidence_ids uuid[],input_hash text,explanation text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT x.id,x.rule_key,0::bigint,x.outcome,x.severity,x.detail_code,ARRAY[]::uuid[],encode(r.input_hash,'hex'),
  'Synthetic preflight diagnostic; not an approved financial rule or readiness decision.'::text
 FROM finance.check_acknowledgment w JOIN finance.check_result x
 ON (x.tenant_id,x.company_id,x.run_id,x.rule_key)=(w.tenant_id,w.company_id,w.run_id,w.rule_key)
 JOIN finance.check_run r ON (r.tenant_id,r.company_id,r.id)=(w.tenant_id,w.company_id,w.run_id)
 JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(r.tenant_id,r.company_id,r.dossier_id)
 WHERE (w.tenant_id,w.company_id,w.actor_id,w.id)=(t,c,a,ack)
  AND (r.dossier_id,r.actor_id,r.dossier_version,ds.owner_id)=(d,a,v,a)
  AND t=finance.scope_tenant() AND c=finance.scope_company() AND a=finance.scope_actor()
  AND EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
   AND m.role='REQUESTER' AND m.active AND m.valid_from<=statement_timestamp()
   AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
$$;
ALTER FUNCTION finance.runtime_warning_receipt(uuid,uuid,uuid,uuid,uuid,bigint) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.runtime_warning_receipt(uuid,uuid,uuid,uuid,uuid,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_warning_receipt(uuid,uuid,uuid,uuid,uuid,bigint) TO finance_runtime;
ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','rule.revise','warning.ack','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide','match.confirm','review.begin','review.decide','membership.create','membership.revoke',
 'risk.resolve','workflow.create','workflow.revise'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','WARNING_ACKNOWLEDGED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','WARNING_ACKNOWLEDGED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
