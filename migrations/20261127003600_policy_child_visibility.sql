-- Permit policy makers to bind rules to their own draft clauses.
-- Keep requester access limited to published clauses.
CREATE POLICY policy_clause_maker_read ON finance.policy_clause
 FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.policy_version p
  WHERE (p.tenant_id,p.company_id,p.id)=(policy_clause.tenant_id,policy_clause.company_id,policy_clause.policy_id)
   AND p.author_id=finance.scope_actor() AND p.state='DRAFT')
 AND EXISTS(SELECT 1 FROM finance.membership m
  WHERE m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
   AND m.actor_id=finance.scope_actor() AND m.role='POLICY_ADMIN' AND m.active
   AND m.valid_from<=statement_timestamp()
   AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 );
-- PostgreSQL UPDATE with a WHERE clause also requires visibility through SELECT
-- policies. These NOLOGIN roles only execute the existing, parent-bound
-- SECURITY DEFINER trigger bodies; runtime receives no writer membership.
CREATE POLICY form_policy_writer_select ON finance.form_version
 FOR SELECT TO finance_form_policy_writer USING(true);
CREATE POLICY template_policy_writer_select ON finance.template_version
 FOR SELECT TO finance_template_policy_writer USING(true);
CREATE POLICY rule_policy_writer_select ON finance.rule_version
 FOR SELECT TO finance_rule_policy_writer USING(true);
