-- E15: immutable approved anonymized corpora and synchronous bounded simulations.
-- Corpus ingestion is intentionally restricted to the trusted offline import authority.
CREATE TABLE finance.simulation_corpus (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,
 maker_id uuid NOT NULL,checker_id uuid NOT NULL,
 facts jsonb NOT NULL CHECK(jsonb_typeof(facts)='array' AND jsonb_array_length(facts) BETWEEN 1 AND 5000 AND octet_length(facts::text)<=4194304),
 corpus_digest bytea NOT NULL CHECK(octet_length(corpus_digest)=32),
 approved_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id),
 FOREIGN KEY(tenant_id,maker_id) REFERENCES finance.identity_subject(tenant_id,id),
 FOREIGN KEY(tenant_id,checker_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(maker_id<>checker_id)
);
-- Compute the digest in the trusted insert guard.
ALTER TABLE finance.simulation_corpus ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.simulation_corpus FORCE ROW LEVEL SECURITY;
CREATE FUNCTION finance.simulation_corpus_approve() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(NEW.tenant_id,NEW.company_id,NEW.maker_id)
   AND m.role='POLICY_ADMIN' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 OR NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(NEW.tenant_id,NEW.company_id,NEW.checker_id)
   AND m.role='FINANCE_APPROVER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'independent corpus approval required' USING ERRCODE='23514'; END IF;
 NEW.corpus_digest:=sha256(convert_to(NEW.facts::text,'UTF8'));
 RETURN NEW;
END $$;
CREATE TRIGGER simulation_corpus_approve BEFORE INSERT ON finance.simulation_corpus FOR EACH ROW EXECUTE FUNCTION finance.simulation_corpus_approve();
CREATE TRIGGER simulation_corpus_immutable BEFORE UPDATE OR DELETE ON finance.simulation_corpus FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
ALTER TABLE finance.rule_simulation ADD COLUMN corpus_id uuid,
 ADD COLUMN comparison jsonb CHECK(comparison IS NULL OR jsonb_typeof(comparison)='object'),
 ADD FOREIGN KEY(tenant_id,company_id,corpus_id) REFERENCES finance.simulation_corpus(tenant_id,company_id,id);
CREATE POLICY simulation_corpus_read ON finance.simulation_corpus FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(finance.scope_tenant(),finance.scope_company(),finance.scope_actor())
 AND m.role='POLICY_ADMIN' AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())));
CREATE POLICY simulation_owner_read ON finance.rule_simulation FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor()
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(finance.scope_tenant(),finance.scope_company(),finance.scope_actor())
 AND m.role='POLICY_ADMIN' AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())));
CREATE POLICY simulation_owner_insert ON finance.rule_simulation FOR INSERT TO finance_runtime WITH CHECK(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor()
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(finance.scope_tenant(),finance.scope_company(),finance.scope_actor())
 AND m.role='POLICY_ADMIN' AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 AND EXISTS(SELECT 1 FROM finance.rule_version r WHERE (r.tenant_id,r.company_id,r.id)=(rule_simulation.tenant_id,rule_simulation.company_id,rule_simulation.rule_id) AND r.author_id=finance.scope_actor())
 AND EXISTS(SELECT 1 FROM finance.simulation_corpus x WHERE (x.tenant_id,x.company_id,x.id,x.corpus_digest)=(rule_simulation.tenant_id,rule_simulation.company_id,rule_simulation.corpus_id,rule_simulation.corpus_digest))
 AND result->>'id'=id::text AND result->>'rule_version_id'=rule_id::text AND result->>'corpus_id'=corpus_id::text);
GRANT SELECT ON finance.simulation_corpus,finance.rule_simulation TO finance_runtime;
GRANT INSERT ON finance.rule_simulation TO finance_runtime;
ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','rule.revise','rule.simulate','warning.ack','dossier.revise','dossier.reopen','dossier.cancel','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide','match.confirm','review.begin','review.decide','membership.create','membership.revoke',
 'risk.resolve','policy.create','policy.submit','policy.publish','policy.retire','policy.revoke','policy.revise','workflow.create','workflow.revise'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','RULE_SIMULATED','WARNING_ACKNOWLEDGED','DOSSIER_REVISED','DOSSIER_REOPENED','DOSSIER_CANCELLED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','POLICY_CREATED','POLICY_SUBMITTED','POLICY_PUBLISHED','POLICY_RETIRED','POLICY_REVOKED','POLICY_REVISED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','RULE_SIMULATED','WARNING_ACKNOWLEDGED','DOSSIER_REVISED','DOSSIER_REOPENED','DOSSIER_CANCELLED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','POLICY_CREATED','POLICY_SUBMITTED','POLICY_PUBLISHED','POLICY_RETIRED','POLICY_REVOKED','POLICY_REVISED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
