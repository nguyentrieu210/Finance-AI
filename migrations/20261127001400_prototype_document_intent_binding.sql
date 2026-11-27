-- Synthetic E18/E05 integration: metadata only, no grant or byte access.
ALTER TABLE finance.dossier DROP CONSTRAINT dossier_version_check,
 ADD CONSTRAINT dossier_version_check CHECK(version BETWEEN 0 AND 9007199254740991);
ALTER TABLE finance.dossier ADD COLUMN initial_updated_at timestamptz;
UPDATE finance.dossier SET initial_updated_at=updated_at;
ALTER TABLE finance.dossier ALTER COLUMN initial_updated_at SET NOT NULL;
CREATE FUNCTION finance.prototype_dossier_version_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='INSERT' THEN
  IF NEW.version<>0 THEN RAISE EXCEPTION 'initial dossier version' USING ERRCODE='23514'; END IF;
  NEW.initial_updated_at:=NEW.updated_at;
 ELSE
  IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.owner_id,NEW.code,NEW.kind,NEW.status,NEW.revision,NEW.initial_updated_at)
   IS DISTINCT FROM (OLD.tenant_id,OLD.company_id,OLD.id,OLD.owner_id,OLD.code,OLD.kind,OLD.status,OLD.revision,OLD.initial_updated_at)
   OR NEW.version<>OLD.version+1 OR NEW.updated_at<OLD.updated_at THEN
    RAISE EXCEPTION 'invalid dossier mutation' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER dossier_version_guard BEFORE INSERT OR UPDATE ON finance.dossier FOR EACH ROW EXECUTE FUNCTION finance.prototype_dossier_version_guard();
CREATE POLICY dossier_owner_update ON finance.dossier FOR UPDATE TO finance_runtime
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND owner_id=finance.scope_actor())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND owner_id=finance.scope_actor());
GRANT UPDATE(version,updated_at) ON finance.dossier TO finance_runtime;
ALTER TABLE finance.upload_intent ADD COLUMN document_kind text CHECK(document_kind ~ '^[A-Z][A-Z0-9_]{0,63}$');
ALTER TABLE finance.upload_intent ADD UNIQUE(tenant_id,company_id,id,actor_id);
CREATE POLICY upload_owner_read ON finance.upload_intent FOR SELECT TO finance_runtime
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor()
 AND EXISTS(SELECT 1 FROM finance.dossier_revision r
  WHERE (r.tenant_id,r.company_id,r.dossier_id,r.id,r.owner_id)=(upload_intent.tenant_id,upload_intent.company_id,upload_intent.dossier_id,upload_intent.revision_id,upload_intent.actor_id)));
CREATE POLICY upload_owner_insert ON finance.upload_intent FOR INSERT TO finance_runtime
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor()
 AND state='PENDING_UPLOAD' AND generation=0 AND document_kind IS NOT NULL
 AND EXISTS(SELECT 1 FROM finance.dossier_revision r JOIN finance.dossier d
  ON (d.tenant_id,d.company_id,d.id,d.owner_id,d.revision)=(r.tenant_id,r.company_id,r.dossier_id,r.owner_id,r.number)
  WHERE (r.tenant_id,r.company_id,r.dossier_id,r.id,r.owner_id)=(upload_intent.tenant_id,upload_intent.company_id,upload_intent.dossier_id,upload_intent.revision_id,upload_intent.actor_id)
  AND d.status='DRAFT'));
GRANT SELECT ON finance.upload_intent TO finance_runtime;
GRANT INSERT(tenant_id,company_id,id,dossier_id,revision_id,actor_id,filename,declared_bytes,declared_sha256,declared_mime,document_kind) ON finance.upload_intent TO finance_runtime;
-- Test owner legacy fixture functions remain unchanged. A new runtime row
-- must have the real dossier binding and a serialized quota check.
CREATE FUNCTION finance.prototype_runtime_intent_guard() RETURNS trigger
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE used bigint;
BEGIN
 IF current_user<>'finance_runtime' THEN RETURN NEW; END IF;
 IF current_setting('transaction_isolation')<>'read committed' THEN
  RAISE EXCEPTION 'read committed required' USING ERRCODE='23514'; END IF;
 PERFORM 1 FROM finance.dossier d JOIN finance.dossier_revision r
 ON (r.tenant_id,r.company_id,r.dossier_id,r.number)=(d.tenant_id,d.company_id,d.id,d.revision)
 WHERE (d.tenant_id,d.company_id,d.id,d.owner_id,r.id)=(NEW.tenant_id,NEW.company_id,NEW.dossier_id,NEW.actor_id,NEW.revision_id)
 AND d.status='DRAFT' FOR UPDATE OF d;
 IF NOT FOUND THEN RAISE EXCEPTION 'invalid dossier binding' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.upload.quota/v1/'||NEW.tenant_id::text||'/'||NEW.company_id::text||'/'||NEW.dossier_id::text,0));
 SELECT COALESCE(sum(declared_bytes),0) INTO used FROM finance.upload_intent
 WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND dossier_id=NEW.dossier_id AND state IN ('PENDING_UPLOAD','FINALIZED');
 IF used+NEW.declared_bytes>104857600 THEN RAISE EXCEPTION 'quota exceeded' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER runtime_intent_guard BEFORE INSERT ON finance.upload_intent FOR EACH ROW EXECUTE FUNCTION finance.prototype_runtime_intent_guard();
ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN ('prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier','prototype.createDocument'));
ALTER TABLE finance.idempotency_record ADD COLUMN document_result_id uuid GENERATED ALWAYS AS
 (CASE WHEN operation_id='prototype.createDocument' THEN result_id ELSE NULL END) STORED;
ALTER TABLE finance.idempotency_record ADD FOREIGN KEY(tenant_id,company_id,document_result_id,actor_id) REFERENCES finance.upload_intent(tenant_id,company_id,id,actor_id);
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN ('ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN ('ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED'));
