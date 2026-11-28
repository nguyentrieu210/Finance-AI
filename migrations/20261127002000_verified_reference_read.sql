-- Read-only application slice for approved, complete, fresh financial references.
-- No draft/revoked snapshot is visible and runtime receives no mutation grant.
CREATE POLICY reference_snapshot_requester_read ON finance.reference_snapshot FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND state='APPROVED' AND fresh_until>statement_timestamp()
 AND EXISTS(SELECT 1 FROM finance.membership m
 JOIN finance.identity_subject s ON (s.tenant_id,s.id)=(m.tenant_id,m.actor_id)
 JOIN finance.company c ON (c.tenant_id,c.id)=(m.tenant_id,m.company_id)
 JOIN finance.tenant t ON t.id=m.tenant_id
 WHERE m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
 AND m.actor_id=finance.scope_actor() AND m.role='REQUESTER' AND m.active
 AND s.active AND c.active AND t.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY financial_resource_requester_read ON finance.financial_resource FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND snapshot_id IS NOT NULL AND coverage_complete AND fresh_until>statement_timestamp()
 AND EXISTS(SELECT 1 FROM finance.reference_snapshot r
 WHERE r.tenant_id=financial_resource.tenant_id AND r.company_id=financial_resource.company_id
 AND r.id=financial_resource.snapshot_id AND r.state='APPROVED' AND r.fresh_until>statement_timestamp())
);
GRANT SELECT ON finance.reference_snapshot,finance.financial_resource TO finance_runtime;
