-- Owner-only synthetic preflight diagnostics. No write grant, no READY transition.
-- Add a persistent result identifier for keyset pagination and the HTTP contract.
ALTER TABLE finance.check_result ADD COLUMN id uuid NOT NULL DEFAULT gen_random_uuid();
ALTER TABLE finance.check_result ADD UNIQUE(tenant_id,company_id,id);
CREATE INDEX check_run_dossier_version ON finance.check_run(tenant_id,company_id,dossier_id,dossier_version);
CREATE POLICY check_run_owner_read ON finance.check_run FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor()
 AND EXISTS(SELECT 1 FROM finance.dossier d WHERE
 (d.tenant_id,d.company_id,d.id)=(check_run.tenant_id,check_run.company_id,check_run.dossier_id)
 AND d.owner_id=finance.scope_actor() AND d.version=check_run.dossier_version)
 AND EXISTS(SELECT 1 FROM finance.membership m JOIN finance.identity_subject s ON
 (s.tenant_id,s.id)=(m.tenant_id,m.actor_id)
 JOIN finance.company c ON (c.tenant_id,c.id)=(m.tenant_id,m.company_id)
 JOIN finance.tenant t ON t.id=m.tenant_id
 WHERE m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
 AND m.actor_id=finance.scope_actor() AND m.role='REQUESTER' AND m.active AND s.active AND c.active AND t.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY check_result_owner_read ON finance.check_result FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.check_run r WHERE
 (r.tenant_id,r.company_id,r.id)=(check_result.tenant_id,check_result.company_id,check_result.run_id))
);
GRANT SELECT ON finance.check_run,finance.check_result TO finance_runtime;
