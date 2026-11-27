-- E18 immutable workflow versions with stable aggregate identity and policy-coupled publication.
ALTER TABLE finance.workflow_version ADD COLUMN workflow_id uuid;
UPDATE finance.workflow_version SET workflow_id=id;
ALTER TABLE finance.workflow_version ALTER COLUMN workflow_id SET NOT NULL;
ALTER TABLE finance.workflow_version ADD CONSTRAINT workflow_version_family UNIQUE(tenant_id,company_id,workflow_id,version);
CREATE INDEX workflow_family_latest ON finance.workflow_version(tenant_id,company_id,workflow_id,version DESC);

CREATE FUNCTION finance.workflow_version_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'immutable workflow version' USING ERRCODE='23514'; END IF;
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.workflow_id,NEW.policy_id,NEW.name,NEW.version,NEW.definition,NEW.author_id)
    IS DISTINCT FROM
    (OLD.tenant_id,OLD.company_id,OLD.id,OLD.workflow_id,OLD.policy_id,OLD.name,OLD.version,OLD.definition,OLD.author_id)
 THEN RAISE EXCEPTION 'immutable workflow content' USING ERRCODE='23514'; END IF;
 IF (OLD.state,NEW.state) NOT IN (('DRAFT','PUBLISHED'),('DRAFT','RETIRED'),('PUBLISHED','RETIRED'))
 THEN RAISE EXCEPTION 'invalid workflow state transition' USING ERRCODE='23514'; END IF;
 IF NEW.state='PUBLISHED' AND (NEW.checker_id IS NULL OR NEW.checker_id=NEW.author_id OR NEW.published_at IS NULL)
 THEN RAISE EXCEPTION 'independent workflow checker required' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER workflow_version_immutable BEFORE UPDATE OR DELETE ON finance.workflow_version
 FOR EACH ROW EXECUTE FUNCTION finance.workflow_version_guard();

CREATE POLICY workflow_admin_read ON finance.workflow_version FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND author_id=finance.scope_actor()
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE
  m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
  AND m.actor_id=finance.scope_actor() AND m.role='POLICY_ADMIN' AND m.active
  AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY workflow_requester_read ON finance.workflow_version FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND state='PUBLISHED'
 AND EXISTS(SELECT 1 FROM finance.policy_version p WHERE
  (p.tenant_id,p.company_id,p.id)=(workflow_version.tenant_id,workflow_version.company_id,workflow_version.policy_id)
  AND p.state='PUBLISHED')
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE
  m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
  AND m.actor_id=finance.scope_actor() AND m.role='REQUESTER' AND m.active
  AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY workflow_admin_insert ON finance.workflow_version FOR INSERT TO finance_runtime WITH CHECK(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND author_id=finance.scope_actor() AND state='DRAFT' AND checker_id IS NULL AND published_at IS NULL
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE
  m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
  AND m.actor_id=finance.scope_actor() AND m.role='POLICY_ADMIN' AND m.active
  AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 AND EXISTS(SELECT 1 FROM finance.policy_version p WHERE
  (p.tenant_id,p.company_id,p.id)=(workflow_version.tenant_id,workflow_version.company_id,workflow_version.policy_id)
  AND p.author_id=finance.scope_actor() AND p.state='DRAFT')
);
GRANT SELECT,INSERT ON finance.workflow_version TO finance_runtime;

CREATE ROLE finance_workflow_policy_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_workflow_policy_writer;
GRANT SELECT ON finance.workflow_version TO finance_workflow_policy_writer;
GRANT UPDATE(state,checker_id,published_at) ON finance.workflow_version TO finance_workflow_policy_writer;
CREATE POLICY workflow_policy_writer_select ON finance.workflow_version FOR SELECT TO finance_workflow_policy_writer USING(true);
CREATE POLICY workflow_policy_writer_update ON finance.workflow_version FOR UPDATE TO finance_workflow_policy_writer
 USING(true) WITH CHECK(state IN ('PUBLISHED','RETIRED'));

CREATE FUNCTION finance.workflow_policy_follow() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF OLD.state='IN_REVIEW' AND NEW.state='PUBLISHED' THEN
  UPDATE finance.workflow_version w SET
   state=CASE WHEN w.version=(
    SELECT max(x.version) FROM finance.workflow_version x
    WHERE x.tenant_id=w.tenant_id AND x.company_id=w.company_id
     AND x.policy_id=w.policy_id AND x.workflow_id=w.workflow_id
   ) THEN 'PUBLISHED' ELSE 'RETIRED' END,
   checker_id=NEW.checker_id,published_at=NEW.published_at
  WHERE w.tenant_id=NEW.tenant_id AND w.company_id=NEW.company_id AND w.policy_id=NEW.id AND w.state='DRAFT';
 ELSIF OLD.state='PUBLISHED' AND NEW.state IN ('RETIRED','REVOKED') THEN
  UPDATE finance.workflow_version SET state='RETIRED'
  WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND policy_id=NEW.id AND state='PUBLISHED';
 END IF;
 RETURN NEW;
END $$;
ALTER FUNCTION finance.workflow_policy_follow() OWNER TO finance_workflow_policy_writer;
REVOKE ALL ON FUNCTION finance.workflow_policy_follow() FROM PUBLIC;
CREATE TRIGGER workflow_policy_state_follow AFTER UPDATE OF state ON finance.policy_version
 FOR EACH ROW EXECUTE FUNCTION finance.workflow_policy_follow();

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide','match.confirm','review.begin','review.decide','membership.create','membership.revoke',
 'risk.resolve','workflow.create','workflow.revise'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
