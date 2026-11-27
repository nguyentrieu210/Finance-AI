-- E21 scoped risk reads and immutable waiver request/decision lifecycle.
ALTER TABLE finance.waiver ADD COLUMN result_id uuid NOT NULL;
ALTER TABLE finance.waiver ADD COLUMN evidence_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[];
ALTER TABLE finance.waiver ADD COLUMN decision_reason text;
ALTER TABLE finance.waiver ADD COLUMN decision_evidence_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[];
ALTER TABLE finance.waiver ADD CONSTRAINT waiver_result_fk
 FOREIGN KEY(tenant_id,company_id,result_id) REFERENCES finance.check_result(tenant_id,company_id,id);
ALTER TABLE finance.waiver ADD CONSTRAINT waiver_result_unique UNIQUE(tenant_id,company_id,result_id);
ALTER TABLE finance.waiver ADD CONSTRAINT waiver_evidence_bounds CHECK(
 cardinality(evidence_ids)<=64 AND cardinality(decision_evidence_ids)<=64
 AND (decision_reason IS NULL OR (octet_length(decision_reason) BETWEEN 1 AND 4096 AND btrim(decision_reason)<>''))
);

CREATE FUNCTION finance.waiver_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'immutable waiver' USING ERRCODE='23514'; END IF;
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.run_id,NEW.rule_key,NEW.dossier_id,NEW.revision_id,NEW.input_hash,NEW.requester_id,NEW.reason,NEW.result_id,NEW.evidence_ids,NEW.created_at)
    IS DISTINCT FROM
    (OLD.tenant_id,OLD.company_id,OLD.id,OLD.run_id,OLD.rule_key,OLD.dossier_id,OLD.revision_id,OLD.input_hash,OLD.requester_id,OLD.reason,OLD.result_id,OLD.evidence_ids,OLD.created_at)
 THEN RAISE EXCEPTION 'immutable waiver binding' USING ERRCODE='23514'; END IF;
 IF OLD.state<>'REQUESTED' OR NEW.state NOT IN ('APPROVED','REJECTED')
 THEN RAISE EXCEPTION 'invalid waiver transition' USING ERRCODE='23514'; END IF;
 IF NEW.approver_id IS NULL OR NEW.approver_id=NEW.requester_id OR NEW.decision_reason IS NULL
 THEN RAISE EXCEPTION 'independent waiver decision required' USING ERRCODE='23514'; END IF;
 IF NEW.state='APPROVED' AND NEW.expires_at<=clock_timestamp()
 THEN RAISE EXCEPTION 'approved waiver must expire in future' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER waiver_immutable BEFORE UPDATE OR DELETE ON finance.waiver
 FOR EACH ROW EXECUTE FUNCTION finance.waiver_guard();

CREATE POLICY waiver_requester_read ON finance.waiver FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND requester_id=finance.scope_actor()
 AND EXISTS(SELECT 1 FROM finance.dossier d WHERE
 (d.tenant_id,d.company_id,d.id)=(waiver.tenant_id,waiver.company_id,waiver.dossier_id)
 AND d.owner_id=finance.scope_actor())
);
CREATE POLICY waiver_approver_read ON finance.waiver FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=finance.scope_tenant()
 AND m.company_id=finance.scope_company() AND m.actor_id=finance.scope_actor()
 AND m.role='FINANCE_APPROVER' AND m.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY waiver_requester_insert ON finance.waiver FOR INSERT TO finance_runtime WITH CHECK(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND requester_id=finance.scope_actor() AND approver_id IS NULL AND state='REQUESTED'
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=finance.scope_tenant()
 AND m.company_id=finance.scope_company() AND m.actor_id=finance.scope_actor()
 AND m.role='REQUESTER' AND m.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 AND EXISTS(SELECT 1 FROM finance.dossier d WHERE
 (d.tenant_id,d.company_id,d.id)=(waiver.tenant_id,waiver.company_id,waiver.dossier_id)
 AND d.owner_id=finance.scope_actor())
);
CREATE POLICY waiver_approver_update ON finance.waiver FOR UPDATE TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND state='REQUESTED'
 AND requester_id<>finance.scope_actor()
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=finance.scope_tenant()
 AND m.company_id=finance.scope_company() AND m.actor_id=finance.scope_actor()
 AND m.role='FINANCE_APPROVER' AND m.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
) WITH CHECK(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND approver_id=finance.scope_actor() AND requester_id<>finance.scope_actor()
 AND state IN ('APPROVED','REJECTED')
);
GRANT SELECT,INSERT ON finance.waiver TO finance_runtime;
GRANT UPDATE(state,approver_id,expires_at,decision_reason,decision_evidence_ids) ON finance.waiver TO finance_runtime;

CREATE POLICY risk_requester_read ON finance.risk_signal FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.dossier_snapshot s JOIN finance.dossier d
 ON (d.tenant_id,d.company_id,d.id)=(s.tenant_id,s.company_id,s.dossier_id)
 WHERE (s.tenant_id,s.company_id,s.id)=(risk_signal.tenant_id,risk_signal.company_id,risk_signal.snapshot_id)
 AND d.owner_id=finance.scope_actor())
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=finance.scope_tenant()
 AND m.company_id=finance.scope_company() AND m.actor_id=finance.scope_actor()
 AND m.role='REQUESTER' AND m.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY risk_decision_requester_read ON finance.risk_decision FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.risk_signal s JOIN finance.dossier_snapshot ds
 ON (ds.tenant_id,ds.company_id,ds.id)=(s.tenant_id,s.company_id,s.snapshot_id)
 JOIN finance.dossier d ON (d.tenant_id,d.company_id,d.id)=(ds.tenant_id,ds.company_id,ds.dossier_id)
 WHERE (s.tenant_id,s.company_id,s.id)=(risk_decision.tenant_id,risk_decision.company_id,risk_decision.signal_id)
 AND d.owner_id=finance.scope_actor())
);
GRANT SELECT ON finance.risk_signal,finance.risk_decision TO finance_runtime;

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','waiver.request','waiver.decide'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED'));
