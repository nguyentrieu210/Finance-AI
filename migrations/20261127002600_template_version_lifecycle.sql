-- E20 immutable template versions backed by CLEAN trusted source bytes.
ALTER TABLE finance.template_version ADD COLUMN template_id uuid;
UPDATE finance.template_version SET template_id=id;
ALTER TABLE finance.template_version ALTER COLUMN template_id SET NOT NULL;
ALTER TABLE finance.template_version ADD COLUMN document_id uuid;
ALTER TABLE finance.template_version ADD CONSTRAINT template_version_family
 UNIQUE(tenant_id,company_id,template_id,version);
ALTER TABLE finance.template_version ADD FOREIGN KEY(tenant_id,company_id,document_id)
 REFERENCES finance.document_blob(tenant_id,company_id,id);
CREATE INDEX template_family_latest ON finance.template_version(tenant_id,company_id,template_id,version DESC);

CREATE FUNCTION finance.template_version_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'immutable template version' USING ERRCODE='23514'; END IF;
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.template_id,NEW.policy_id,NEW.name,NEW.version,
     NEW.format,NEW.source_digest,NEW.document_id,NEW.author_id)
    IS DISTINCT FROM
    (OLD.tenant_id,OLD.company_id,OLD.id,OLD.template_id,OLD.policy_id,OLD.name,OLD.version,
     OLD.format,OLD.source_digest,OLD.document_id,OLD.author_id)
 THEN RAISE EXCEPTION 'immutable template content' USING ERRCODE='23514'; END IF;
 IF (OLD.state,NEW.state) NOT IN (('DRAFT','PUBLISHED'),('DRAFT','RETIRED'),('PUBLISHED','RETIRED'))
 THEN RAISE EXCEPTION 'invalid template state transition' USING ERRCODE='23514'; END IF;
 IF NEW.state='PUBLISHED' AND
   (NEW.checker_id IS NULL OR NEW.checker_id=NEW.author_id OR NEW.published_at IS NULL)
 THEN RAISE EXCEPTION 'independent template checker required' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER template_version_immutable BEFORE UPDATE OR DELETE ON finance.template_version
 FOR EACH ROW EXECUTE FUNCTION finance.template_version_guard();

CREATE POLICY template_admin_read ON finance.template_version FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND author_id=finance.scope_actor()
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE
   m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
   AND m.actor_id=finance.scope_actor() AND m.role='POLICY_ADMIN' AND m.active
   AND m.valid_from<=statement_timestamp()
   AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY template_requester_read ON finance.template_version FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND state='PUBLISHED'
 AND EXISTS(SELECT 1 FROM finance.policy_version p WHERE
   (p.tenant_id,p.company_id,p.id)=(template_version.tenant_id,template_version.company_id,template_version.policy_id)
   AND p.state='PUBLISHED')
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE
   m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
   AND m.actor_id=finance.scope_actor() AND m.role='REQUESTER' AND m.active
   AND m.valid_from<=statement_timestamp()
   AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY template_admin_insert ON finance.template_version FOR INSERT TO finance_runtime WITH CHECK(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND author_id=finance.scope_actor() AND state='DRAFT' AND checker_id IS NULL AND published_at IS NULL
 AND EXISTS(SELECT 1 FROM finance.policy_version p WHERE
   (p.tenant_id,p.company_id,p.id)=(template_version.tenant_id,template_version.company_id,template_version.policy_id)
   AND p.author_id=finance.scope_actor() AND p.state='DRAFT')
);
GRANT SELECT,INSERT ON finance.template_version TO finance_runtime;

CREATE ROLE finance_template_source_reader NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_template_source_reader;
GRANT SELECT ON finance.document_blob,finance.membership TO finance_template_source_reader;
CREATE POLICY template_source_blob_read ON finance.document_blob FOR SELECT TO finance_template_source_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND scan_state='CLEAN');
CREATE POLICY template_source_membership_read ON finance.membership FOR SELECT TO finance_template_source_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());

CREATE FUNCTION finance.template_source_digest(doc uuid,fmt text) RETURNS bytea
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE b finance.document_blob%ROWTYPE; expected text;
BEGIN
 IF doc IS NULL OR fmt NOT IN ('DOCX','XLSX','PDF') OR finance.scope_actor() IS NULL
 OR NOT EXISTS(SELECT 1 FROM finance.membership m WHERE
   m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
   AND m.actor_id=finance.scope_actor() AND m.role='POLICY_ADMIN' AND m.active
   AND m.valid_from<=statement_timestamp()
   AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 THEN RAISE EXCEPTION 'template source forbidden' USING ERRCODE='42501'; END IF;
 expected:=CASE fmt
  WHEN 'DOCX' THEN 'application/vnd.openxmlformats-officedocument.wordprocessingml.document'
  WHEN 'XLSX' THEN 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'
  ELSE 'application/pdf' END;
 SELECT * INTO b FROM finance.document_blob WHERE tenant_id=finance.scope_tenant()
  AND company_id=finance.scope_company() AND id=doc AND scan_state='CLEAN';
 IF NOT FOUND OR b.mime<>expected THEN
  RAISE EXCEPTION 'invalid clean template source' USING ERRCODE='23514'; END IF;
 IF fmt IN ('DOCX','XLSX') AND position(convert_to('vbaProject.bin','UTF8') in b.synthetic_bytes)>0
 THEN RAISE EXCEPTION 'macro-enabled template forbidden' USING ERRCODE='23514'; END IF;
 RETURN b.sha256_digest;
END $$;
ALTER FUNCTION finance.template_source_digest(uuid,text) OWNER TO finance_template_source_reader;
REVOKE ALL ON FUNCTION finance.template_source_digest(uuid,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.template_source_digest(uuid,text) TO finance_runtime;

CREATE ROLE finance_template_policy_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_template_policy_writer;
GRANT SELECT ON finance.template_version TO finance_template_policy_writer;
GRANT UPDATE(state,checker_id,published_at) ON finance.template_version TO finance_template_policy_writer;
CREATE POLICY template_policy_writer_update ON finance.template_version FOR UPDATE TO finance_template_policy_writer
 USING(true) WITH CHECK(state IN ('PUBLISHED','RETIRED'));
CREATE FUNCTION finance.template_policy_follow() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF OLD.state='IN_REVIEW' AND NEW.state='PUBLISHED' THEN
   UPDATE finance.template_version x SET
    state=CASE WHEN x.version=(
      SELECT max(y.version) FROM finance.template_version y
      WHERE y.tenant_id=x.tenant_id AND y.company_id=x.company_id
      AND y.policy_id=x.policy_id AND y.template_id=x.template_id
    ) THEN 'PUBLISHED' ELSE 'RETIRED' END,
    checker_id=NEW.checker_id,published_at=NEW.published_at
   WHERE x.tenant_id=NEW.tenant_id AND x.company_id=NEW.company_id
     AND x.policy_id=NEW.id AND x.state='DRAFT';
 ELSIF OLD.state='PUBLISHED' AND NEW.state IN ('RETIRED','REVOKED') THEN
   UPDATE finance.template_version SET state='RETIRED'
   WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id
     AND policy_id=NEW.id AND state='PUBLISHED';
 END IF;
 RETURN NEW;
END $$;
ALTER FUNCTION finance.template_policy_follow() OWNER TO finance_template_policy_writer;
REVOKE ALL ON FUNCTION finance.template_policy_follow() FROM PUBLIC;
CREATE TRIGGER template_policy_state_follow AFTER UPDATE OF state ON finance.policy_version
 FOR EACH ROW EXECUTE FUNCTION finance.template_policy_follow();

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED'));
