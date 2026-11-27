-- E19 immutable form versions with stable aggregate identity and policy-coupled publication.
ALTER TABLE finance.form_version ADD COLUMN form_id uuid;
UPDATE finance.form_version SET form_id=id;
ALTER TABLE finance.form_version ALTER COLUMN form_id SET NOT NULL;
ALTER TABLE finance.form_version ADD CONSTRAINT form_version_family UNIQUE(tenant_id,company_id,form_id,version);
CREATE INDEX form_family_latest ON finance.form_version(tenant_id,company_id,form_id,version DESC);

CREATE FUNCTION finance.form_version_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'immutable form version' USING ERRCODE='23514'; END IF;
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.form_id,NEW.policy_id,NEW.name,NEW.version,NEW.schema_json,NEW.author_id)
    IS DISTINCT FROM
    (OLD.tenant_id,OLD.company_id,OLD.id,OLD.form_id,OLD.policy_id,OLD.name,OLD.version,OLD.schema_json,OLD.author_id)
 THEN RAISE EXCEPTION 'immutable form content' USING ERRCODE='23514'; END IF;
 IF (OLD.state,NEW.state) NOT IN (('DRAFT','PUBLISHED'),('DRAFT','RETIRED'),('PUBLISHED','RETIRED'))
 THEN RAISE EXCEPTION 'invalid form state transition' USING ERRCODE='23514'; END IF;
 IF NEW.state='PUBLISHED' AND (NEW.checker_id IS NULL OR NEW.checker_id=NEW.author_id OR NEW.published_at IS NULL)
 THEN RAISE EXCEPTION 'independent form checker required' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER form_version_immutable BEFORE UPDATE OR DELETE ON finance.form_version
 FOR EACH ROW EXECUTE FUNCTION finance.form_version_guard();

CREATE POLICY policy_admin_draft_read ON finance.policy_version FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND author_id=finance.scope_actor() AND state IN ('DRAFT','IN_REVIEW')
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE
 m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
 AND m.actor_id=finance.scope_actor() AND m.role='POLICY_ADMIN' AND m.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY form_admin_read ON finance.form_version FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND author_id=finance.scope_actor()
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE
 m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
 AND m.actor_id=finance.scope_actor() AND m.role='POLICY_ADMIN' AND m.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY form_requester_read ON finance.form_version FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND state='PUBLISHED'
 AND EXISTS(SELECT 1 FROM finance.policy_version p WHERE
 (p.tenant_id,p.company_id,p.id)=(form_version.tenant_id,form_version.company_id,form_version.policy_id)
 AND p.state='PUBLISHED')
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE
 m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
 AND m.actor_id=finance.scope_actor() AND m.role='REQUESTER' AND m.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY form_admin_insert ON finance.form_version FOR INSERT TO finance_runtime WITH CHECK(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND author_id=finance.scope_actor() AND state='DRAFT' AND checker_id IS NULL AND published_at IS NULL
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE
 m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
 AND m.actor_id=finance.scope_actor() AND m.role='POLICY_ADMIN' AND m.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 AND EXISTS(SELECT 1 FROM finance.policy_version p WHERE
 (p.tenant_id,p.company_id,p.id)=(form_version.tenant_id,form_version.company_id,form_version.policy_id)
 AND p.author_id=finance.scope_actor() AND p.state='DRAFT')
);
GRANT SELECT,INSERT ON finance.form_version TO finance_runtime;

CREATE ROLE finance_form_policy_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_form_policy_writer;
GRANT SELECT ON finance.form_version TO finance_form_policy_writer;
GRANT UPDATE(state,checker_id,published_at) ON finance.form_version TO finance_form_policy_writer;
CREATE POLICY form_policy_writer_update ON finance.form_version FOR UPDATE TO finance_form_policy_writer
 USING(true) WITH CHECK(state IN ('PUBLISHED','RETIRED'));

CREATE FUNCTION finance.form_policy_follow() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF OLD.state='IN_REVIEW' AND NEW.state='PUBLISHED' THEN
   UPDATE finance.form_version f SET
    state=CASE WHEN f.version=(
      SELECT max(x.version) FROM finance.form_version x
      WHERE x.tenant_id=f.tenant_id AND x.company_id=f.company_id AND x.policy_id=f.policy_id AND x.form_id=f.form_id
    ) THEN 'PUBLISHED' ELSE 'RETIRED' END,
    checker_id=NEW.checker_id,published_at=NEW.published_at
   WHERE f.tenant_id=NEW.tenant_id AND f.company_id=NEW.company_id AND f.policy_id=NEW.id AND f.state='DRAFT';
 ELSIF OLD.state='PUBLISHED' AND NEW.state IN ('RETIRED','REVOKED') THEN
   UPDATE finance.form_version SET state='RETIRED'
   WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND policy_id=NEW.id AND state='PUBLISHED';
 END IF;
 RETURN NEW;
END $$;
ALTER FUNCTION finance.form_policy_follow() OWNER TO finance_form_policy_writer;
REVOKE ALL ON FUNCTION finance.form_policy_follow() FROM PUBLIC;
CREATE TRIGGER form_policy_state_follow AFTER UPDATE OF state ON finance.policy_version
 FOR EACH ROW EXECUTE FUNCTION finance.form_policy_follow();

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier','prototype.createDocument',
 'form.create','form.revise'));

ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED'));
