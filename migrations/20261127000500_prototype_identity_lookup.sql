-- Dedicated read-only synthetic identity resolver. Never use migration credentials for identity lookup.
CREATE ROLE finance_identity_lookup NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_identity_lookup;
CREATE POLICY identity_lookup_tenant ON finance.tenant FOR SELECT TO finance_identity_lookup USING(id=finance.scope_tenant());
CREATE POLICY identity_lookup_company ON finance.company FOR SELECT TO finance_identity_lookup USING(tenant_id=finance.scope_tenant() AND id=finance.scope_company());
CREATE POLICY identity_lookup_subject ON finance.identity_subject FOR SELECT TO finance_identity_lookup USING(
 tenant_id=finance.scope_tenant() AND issuer=NULLIF(current_setting('finance.identity_issuer',true),'')
 AND subject=NULLIF(current_setting('finance.identity_subject',true),'')
);
CREATE POLICY identity_lookup_membership ON finance.membership FOR SELECT TO finance_identity_lookup USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.identity_subject i WHERE (i.tenant_id,i.id)=(membership.tenant_id,membership.actor_id))
);
GRANT SELECT ON finance.tenant,finance.company,finance.identity_subject,finance.membership TO finance_identity_lookup;
-- No provisioning, approval, audit or organization writes. GUCs set only by trusted verified-token code.
