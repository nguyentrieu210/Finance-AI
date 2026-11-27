-- E17 runtime read: published/effective policy knowledge only. No ingestion grant.
CREATE POLICY policy_clause_requester_read ON finance.policy_clause FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.policy_version p
   WHERE (p.tenant_id,p.company_id,p.id)=(policy_clause.tenant_id,policy_clause.company_id,policy_clause.policy_id)
   AND p.state='PUBLISHED')
 AND EXISTS(SELECT 1 FROM finance.membership m
   JOIN finance.identity_subject s ON (s.tenant_id,s.id)=(m.tenant_id,m.actor_id)
   WHERE m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company()
   AND m.actor_id=finance.scope_actor() AND m.role='REQUESTER' AND m.active AND s.active
   AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY knowledge_chunk_requester_read ON finance.knowledge_chunk FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.policy_version p
   WHERE (p.tenant_id,p.company_id,p.id)=(knowledge_chunk.tenant_id,knowledge_chunk.company_id,knowledge_chunk.policy_id)
   AND p.state='PUBLISHED')
);
GRANT SELECT ON finance.policy_clause,finance.knowledge_chunk TO finance_runtime;
