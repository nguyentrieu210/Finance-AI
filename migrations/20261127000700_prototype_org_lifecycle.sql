-- Synthetic-only organization hierarchy lifecycle.
ALTER TABLE finance.organization_unit ADD COLUMN version bigint NOT NULL DEFAULT 0 CHECK(version BETWEEN 0 AND 9007199254740991);
ALTER TABLE finance.idempotency_record ADD COLUMN result_version bigint NOT NULL DEFAULT 0 CHECK(result_version BETWEEN 0 AND 9007199254740991);
ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN ('prototype.createOrganizationUnit','prototype.updateOrganizationUnit'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN ('ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN ('ORG_UNIT_CREATED','ORG_UNIT_UPDATED'));
CREATE INDEX organization_parent ON finance.organization_unit(tenant_id,company_id,parent_id);
CREATE POLICY unit_update ON finance.organization_unit FOR UPDATE TO finance_runtime
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
GRANT UPDATE(parent_id,name,active,version) ON finance.organization_unit TO finance_runtime;
CREATE FUNCTION finance.lock_org_tree(t uuid,c uuid) RETURNS void LANGUAGE sql VOLATILE SET search_path=pg_catalog AS $$
 SELECT pg_advisory_xact_lock(hashtextextended('finance.org.tree/v1/'||t::text||'/'||c::text,0))
$$;
REVOKE ALL ON FUNCTION finance.lock_org_tree(uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.lock_org_tree(uuid,uuid) TO finance_runtime;
CREATE FUNCTION finance.validate_org_tree() RETURNS trigger LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
BEGIN
 IF current_setting('transaction_isolation') <> 'read committed' THEN
  RAISE EXCEPTION 'organization mutation requires read committed' USING ERRCODE='25000';
 END IF;
 PERFORM finance.lock_org_tree(NEW.tenant_id,NEW.company_id);
 IF TG_OP='INSERT' THEN
  IF NEW.version<>0 THEN RAISE EXCEPTION 'invalid initial version' USING ERRCODE='23514'; END IF;
 ELSE
  IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.code,NEW.kind) IS DISTINCT FROM (OLD.tenant_id,OLD.company_id,OLD.id,OLD.code,OLD.kind)
     OR NEW.version<>OLD.version+1 THEN
   RAISE EXCEPTION 'immutable identity or invalid version' USING ERRCODE='23514';
  END IF;
 END IF;
 IF NEW.parent_id IS NOT NULL THEN
  IF EXISTS (
   WITH RECURSIVE ancestors(id,parent_id) AS (
    SELECT id,parent_id FROM finance.organization_unit WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND id=NEW.parent_id
    UNION
    SELECT u.id,u.parent_id FROM finance.organization_unit u JOIN ancestors a ON u.id=a.parent_id
    WHERE u.tenant_id=NEW.tenant_id AND u.company_id=NEW.company_id
   ) SELECT 1 FROM ancestors WHERE id=NEW.id
  ) OR NEW.parent_id=NEW.id THEN
   RAISE EXCEPTION 'organization cycle' USING ERRCODE='23514';
  END IF;
  IF NEW.active AND EXISTS(SELECT 1 FROM finance.organization_unit WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND id=NEW.parent_id AND NOT active) THEN
   RAISE EXCEPTION 'inactive parent' USING ERRCODE='23514';
  END IF;
 END IF;
 IF NOT NEW.active AND EXISTS(SELECT 1 FROM finance.organization_unit WHERE tenant_id=NEW.tenant_id AND company_id=NEW.company_id AND parent_id=NEW.id AND active) THEN
  RAISE EXCEPTION 'active child' USING ERRCODE='23514';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER validate_org_tree BEFORE INSERT OR UPDATE ON finance.organization_unit FOR EACH ROW EXECUTE FUNCTION finance.validate_org_tree();
-- Existing scoped FK catches missing parent; no deletion, code/kind/scope changes or cascading deactivation.
