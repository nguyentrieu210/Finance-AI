-- E15 immutable rule drafts, policy-coupled publication, and scoped reads.
CREATE FUNCTION finance.rule_version_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'immutable rule version' USING ERRCODE='23514'; END IF;
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.policy_id,NEW.clause_id,NEW.rule_key,NEW.version,NEW.severity,NEW.waivable,NEW.definition,NEW.author_id)
    IS DISTINCT FROM
    (OLD.tenant_id,OLD.company_id,OLD.id,OLD.policy_id,OLD.clause_id,OLD.rule_key,OLD.version,OLD.severity,OLD.waivable,OLD.definition,OLD.author_id)
 THEN RAISE EXCEPTION 'immutable rule content' USING ERRCODE='23514'; END IF;
 IF (OLD.state,NEW.state) NOT IN (('DRAFT','PUBLISHED'),('DRAFT','RETIRED'),('PUBLISHED','RETIRED'))
 THEN RAISE EXCEPTION 'invalid rule state transition' USING ERRCODE='23514'; END IF;
 IF NEW.state='PUBLISHED' AND (NEW.checker_id IS NULL OR NEW.checker_id=NEW.author_id OR NEW.published_at IS NULL)
 THEN RAISE EXCEPTION 'independent rule checker required' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER rule_version_immutable BEFORE UPDATE OR DELETE ON finance.rule_version
 FOR EACH ROW EXECUTE FUNCTION finance.rule_version_guard();

CREATE POLICY rule_admin_read ON finance.rule_version FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND author_id=finance.scope_actor()
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE
 m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
 AND m.actor_id=finance.scope_actor() AND m.role='POLICY_ADMIN' AND m.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY rule_requester_read ON finance.rule_version FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND state='PUBLISHED'
 AND EXISTS(SELECT 1 FROM finance.policy_version p WHERE
 (p.tenant_id,p.company_id,p.id)=(rule_version.tenant_id,rule_version.company_id,rule_version.policy_id)
 AND p.state='PUBLISHED')
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE
 m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
 AND m.actor_id=finance.scope_actor() AND m.role='REQUESTER' AND m.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY rule_admin_insert ON finance.rule_version FOR INSERT TO finance_runtime WITH CHECK(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND author_id=finance.scope_actor() AND state='DRAFT' AND checker_id IS NULL AND published_at IS NULL
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE
 m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
 AND m.actor_id=finance.scope_actor() AND m.role='POLICY_ADMIN' AND m.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 AND EXISTS(SELECT 1 FROM finance.policy_version p WHERE
 (p.tenant_id,p.company_id,p.id)=(rule_version.tenant_id,rule_version.company_id,rule_version.policy_id)
 AND p.author_id=finance.scope_actor() AND p.state='DRAFT')
 AND EXISTS(SELECT 1 FROM finance.policy_clause cl WHERE
 (cl.tenant_id,cl.company_id,cl.policy_id,cl.id)=(rule_version.tenant_id,rule_version.company_id,rule_version.policy_id,rule_version.clause_id))
);
GRANT SELECT,INSERT ON finance.rule_version TO finance_runtime;

CREATE ROLE finance_rule_policy_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_rule_policy_writer;
GRANT SELECT ON finance.rule_version TO finance_rule_policy_writer;
GRANT UPDATE(state,checker_id,published_at) ON finance.rule_version TO finance_rule_policy_writer;
CREATE POLICY rule_policy_writer_update ON finance.rule_version FOR UPDATE TO finance_rule_policy_writer
 USING(true) WITH CHECK(state IN ('PUBLISHED','RETIRED'));

CREATE FUNCTION finance.rule_policy_follow() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF OLD.state='IN_REVIEW' AND NEW.state='PUBLISHED' THEN
   UPDATE finance.rule_version SET state='PUBLISHED',checker_id=NEW.checker_id,published_at=NEW.published_at
   WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND policy_id=NEW.id AND state='DRAFT';
 ELSIF OLD.state='PUBLISHED' AND NEW.state IN ('RETIRED','REVOKED') THEN
   UPDATE finance.rule_version SET state='RETIRED'
   WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND policy_id=NEW.id AND state='PUBLISHED';
 END IF;
 RETURN NEW;
END $$;
ALTER FUNCTION finance.rule_policy_follow() OWNER TO finance_rule_policy_writer;
REVOKE ALL ON FUNCTION finance.rule_policy_follow() FROM PUBLIC;
CREATE TRIGGER rule_policy_state_follow AFTER UPDATE OF state ON finance.policy_version
 FOR EACH ROW EXECUTE FUNCTION finance.rule_policy_follow();

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready','rule.create'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED'));
