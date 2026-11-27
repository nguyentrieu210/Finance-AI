-- E15 stable rule identity and immutable revision reason.
ALTER TABLE finance.rule_version ADD COLUMN rule_id uuid, ADD COLUMN revision_reason text;
ALTER TABLE finance.rule_version DISABLE TRIGGER rule_version_immutable;
UPDATE finance.rule_version r SET rule_id=(
 SELECT x.id FROM finance.rule_version x WHERE x.tenant_id=r.tenant_id AND x.company_id=r.company_id
  AND x.rule_key=r.rule_key ORDER BY x.version LIMIT 1
);
ALTER TABLE finance.rule_version ENABLE TRIGGER rule_version_immutable;
ALTER TABLE finance.rule_version ALTER COLUMN rule_id SET NOT NULL;
ALTER TABLE finance.rule_version ADD CONSTRAINT rule_family_unique UNIQUE(tenant_id,company_id,rule_id,version),
 ADD CONSTRAINT rule_family_root FOREIGN KEY(tenant_id,company_id,rule_id) REFERENCES finance.rule_version(tenant_id,company_id,id),
 ADD CONSTRAINT rule_revision_reason_check CHECK(revision_reason IS NULL OR (octet_length(revision_reason) BETWEEN 1 AND 2048 AND btrim(revision_reason)<>''));
CREATE FUNCTION finance.rule_family_insert_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF NEW.version=1 THEN
  IF NEW.rule_id IS NOT NULL AND NEW.rule_id<>NEW.id THEN RAISE EXCEPTION 'invalid rule root' USING ERRCODE='23514'; END IF;
  NEW.rule_id:=NEW.id;
 ELSIF NEW.rule_id IS NULL OR NEW.revision_reason IS NULL OR NOT EXISTS(
  SELECT 1 FROM finance.rule_version p WHERE p.tenant_id=NEW.tenant_id AND p.company_id=NEW.company_id
   AND p.rule_id=NEW.rule_id AND p.version=NEW.version-1 AND p.rule_key=NEW.rule_key
   AND p.policy_id=NEW.policy_id AND p.author_id=NEW.author_id AND p.state='DRAFT'
 ) THEN RAISE EXCEPTION 'invalid rule revision' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER rule_family_insert BEFORE INSERT ON finance.rule_version
 FOR EACH ROW EXECUTE FUNCTION finance.rule_family_insert_guard();
-- E15 immutable rule drafts, policy-coupled publication, and scoped reads.
CREATE OR REPLACE FUNCTION finance.rule_version_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'immutable rule version' USING ERRCODE='23514'; END IF;
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.rule_id,NEW.revision_reason,NEW.policy_id,NEW.clause_id,NEW.rule_key,NEW.version,NEW.severity,NEW.waivable,NEW.definition,NEW.author_id)
    IS DISTINCT FROM
    (OLD.tenant_id,OLD.company_id,OLD.id,OLD.rule_id,OLD.revision_reason,OLD.policy_id,OLD.clause_id,OLD.rule_key,OLD.version,OLD.severity,OLD.waivable,OLD.definition,OLD.author_id)
 THEN RAISE EXCEPTION 'immutable rule content' USING ERRCODE='23514'; END IF;
 IF (OLD.state,NEW.state) NOT IN (('DRAFT','PUBLISHED'),('DRAFT','RETIRED'),('PUBLISHED','RETIRED'))
 THEN RAISE EXCEPTION 'invalid rule state transition' USING ERRCODE='23514'; END IF;
 IF NEW.state='PUBLISHED' AND (NEW.checker_id IS NULL OR NEW.checker_id=NEW.author_id OR NEW.published_at IS NULL)
 THEN RAISE EXCEPTION 'independent rule checker required' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION finance.rule_policy_follow() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF OLD.state='IN_REVIEW' AND NEW.state='PUBLISHED' THEN
  UPDATE finance.rule_version r SET
   state=CASE WHEN r.version=(SELECT max(x.version) FROM finance.rule_version x
     WHERE (x.tenant_id,x.company_id,x.rule_id,x.policy_id)=(r.tenant_id,r.company_id,r.rule_id,r.policy_id))
    THEN 'PUBLISHED' ELSE 'RETIRED' END,
   checker_id=NEW.checker_id,published_at=NEW.published_at
  WHERE r.tenant_id=NEW.tenant_id AND r.company_id=NEW.company_id AND r.policy_id=NEW.id AND r.state='DRAFT';
 ELSIF OLD.state='PUBLISHED' AND NEW.state IN ('RETIRED','REVOKED') THEN
  UPDATE finance.rule_version SET state='RETIRED'
  WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND policy_id=NEW.id AND state='PUBLISHED';
 END IF;
 RETURN NEW;
END $$;
ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','rule.revise','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide','match.confirm','review.begin','review.decide','membership.create','membership.revoke',
 'risk.resolve','workflow.create','workflow.revise'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
