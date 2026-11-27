-- Runtime read slice: only currently PUBLISHED policy versions in authenticated scope.
CREATE POLICY policy_published_requester_read ON finance.policy_version FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND state='PUBLISHED'
 AND EXISTS(SELECT 1 FROM finance.membership m
 JOIN finance.identity_subject s ON (s.tenant_id,s.id)=(m.tenant_id,m.actor_id)
 JOIN finance.company c ON (c.tenant_id,c.id)=(m.tenant_id,m.company_id)
 JOIN finance.tenant t ON t.id=m.tenant_id
 WHERE m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
 AND m.actor_id=finance.scope_actor() AND m.role='REQUESTER' AND m.active
 AND s.active AND c.active AND t.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
GRANT SELECT ON finance.policy_version TO finance_runtime;
