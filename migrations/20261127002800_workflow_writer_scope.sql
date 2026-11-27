-- Complete E18 workflow writer authority used by SECURITY DEFINER command functions.
GRANT SELECT ON finance.membership TO finance_workflow_writer;
CREATE POLICY workflow_membership_read ON finance.membership FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
  AND actor_id=finance.scope_actor());
