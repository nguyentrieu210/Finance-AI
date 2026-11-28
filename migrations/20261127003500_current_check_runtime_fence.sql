-- Defense-in-depth current-version fence for requester preflight diagnostics.
-- Workflow writers intentionally have broader scoped reads inside SECURITY DEFINER
-- transitions; finance_runtime must never inherit a stale check run/result through
-- permissive-policy composition.
CREATE POLICY check_run_runtime_current_fence
 ON finance.check_run AS RESTRICTIVE
 FOR SELECT TO finance_runtime
 USING(
  tenant_id=finance.scope_tenant()
  AND company_id=finance.scope_company()
  AND actor_id=finance.scope_actor()
  AND EXISTS(
   SELECT 1
   FROM finance.dossier d
   WHERE (d.tenant_id,d.company_id,d.id)=(
    check_run.tenant_id,check_run.company_id,check_run.dossier_id
   )
   AND d.owner_id=finance.scope_actor()
   AND d.version=check_run.dossier_version
  )
 );

CREATE POLICY check_result_runtime_current_fence
 ON finance.check_result AS RESTRICTIVE
 FOR SELECT TO finance_runtime
 USING(
  tenant_id=finance.scope_tenant()
  AND company_id=finance.scope_company()
  AND EXISTS(
   SELECT 1
   FROM finance.check_run r
   JOIN finance.dossier d
    ON (d.tenant_id,d.company_id,d.id)=(
     r.tenant_id,r.company_id,r.dossier_id
    )
   WHERE (r.tenant_id,r.company_id,r.id)=(
    check_result.tenant_id,check_result.company_id,check_result.run_id
   )
   AND r.actor_id=finance.scope_actor()
   AND d.owner_id=finance.scope_actor()
   AND d.version=r.dossier_version
  )
 );
