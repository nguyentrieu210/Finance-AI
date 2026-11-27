-- Prototype: lock current grants before mutation; no public login or grant management.
CREATE ROLE finance_permission_guard NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_permission_guard;
GRANT SELECT ON finance.tenant,finance.company,finance.identity_subject,finance.membership TO finance_permission_guard;
-- PostgreSQL row locking requires UPDATE privilege; the non-login definer only executes SELECT.
GRANT UPDATE(active) ON finance.tenant,finance.company,finance.identity_subject,finance.membership TO finance_permission_guard;
CREATE POLICY guard_tenant_read ON finance.tenant FOR SELECT TO finance_permission_guard USING(id=finance.scope_tenant());
CREATE POLICY guard_company_read ON finance.company FOR SELECT TO finance_permission_guard USING(tenant_id=finance.scope_tenant() AND id=finance.scope_company());
CREATE POLICY guard_subject_read ON finance.identity_subject FOR SELECT TO finance_permission_guard USING(tenant_id=finance.scope_tenant() AND id=finance.scope_actor());
CREATE POLICY guard_membership_read ON finance.membership FOR SELECT TO finance_permission_guard USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY guard_tenant_lock ON finance.tenant FOR UPDATE TO finance_permission_guard USING(id=finance.scope_tenant()) WITH CHECK(false);
CREATE POLICY guard_company_lock ON finance.company FOR UPDATE TO finance_permission_guard USING(tenant_id=finance.scope_tenant() AND id=finance.scope_company()) WITH CHECK(false);
CREATE POLICY guard_subject_lock ON finance.identity_subject FOR UPDATE TO finance_permission_guard USING(tenant_id=finance.scope_tenant() AND id=finance.scope_actor()) WITH CHECK(false);
CREATE POLICY guard_membership_lock ON finance.membership FOR UPDATE TO finance_permission_guard USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor()) WITH CHECK(false);
CREATE FUNCTION finance.lock_current_roles() RETURNS text[]
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE t uuid:=finance.scope_tenant(); c uuid:=finance.scope_company(); a uuid:=finance.scope_actor();
 enabled boolean; roles text[];
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL THEN RETURN ARRAY[]::text[]; END IF;
 SELECT active INTO enabled FROM finance.tenant WHERE id=t FOR SHARE;
 IF enabled IS DISTINCT FROM true THEN RETURN ARRAY[]::text[]; END IF;
 SELECT active INTO enabled FROM finance.company WHERE tenant_id=t AND id=c FOR SHARE;
 IF enabled IS DISTINCT FROM true THEN RETURN ARRAY[]::text[]; END IF;
 SELECT active INTO enabled FROM finance.identity_subject WHERE tenant_id=t AND id=a FOR SHARE;
 IF enabled IS DISTINCT FROM true THEN RETURN ARRAY[]::text[]; END IF;
 -- Lock even inactive rows, deterministically. Re-evaluate after waiting on any concurrent revoke.
 PERFORM 1 FROM finance.membership WHERE tenant_id=t AND company_id=c AND actor_id=a ORDER BY role FOR SHARE;
 SELECT COALESCE(array_agg(role ORDER BY role),ARRAY[]::text[]) INTO roles FROM finance.membership
 WHERE tenant_id=t AND company_id=c AND actor_id=a AND active
 AND valid_from<=clock_timestamp() AND (valid_until IS NULL OR valid_until>clock_timestamp());
 RETURN roles;
END $$;
ALTER FUNCTION finance.lock_current_roles() OWNER TO finance_permission_guard;
REVOKE ALL ON FUNCTION finance.lock_current_roles() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.lock_current_roles() TO finance_runtime;
-- Do not grant membership in guard role to runtime. Existing row locks last through caller transaction.
