-- E01 scoped job receipt. A job is visible only to the actor who issued
-- its audited command. Undispatched events remain QUEUED, never SUCCEEDED.
CREATE ROLE finance_job_reader NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_job_reader;
GRANT SELECT ON finance.outbox,finance.job,finance.audit_event,finance.membership TO finance_job_reader;
CREATE POLICY job_reader_outbox ON finance.outbox FOR SELECT TO finance_job_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY job_reader_job ON finance.job FOR SELECT TO finance_job_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY job_reader_audit ON finance.audit_event FOR SELECT TO finance_job_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY job_reader_membership ON finance.membership FOR SELECT TO finance_job_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());

CREATE FUNCTION finance.runtime_job_status(t uuid,c uuid,a uuid,j uuid)
RETURNS TABLE(id uuid,state text,attempt bigint,progress bigint,error_code text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT o.id,
  CASE x.state WHEN 'LEASED' THEN 'RUNNING' WHEN 'SUCCEEDED' THEN 'SUCCEEDED'
   WHEN 'DEAD' THEN 'DEAD_LETTER' ELSE 'QUEUED' END,
  COALESCE(x.attempts,0)::bigint,
  CASE WHEN x.state='SUCCEEDED' THEN 100 ELSE 0 END::bigint,
  CASE WHEN x.state='DEAD' THEN x.last_error ELSE NULL END
 FROM finance.outbox o
 LEFT JOIN finance.job x ON (x.tenant_id,x.company_id,x.id)=(o.tenant_id,o.company_id,o.id)
 WHERE t=finance.scope_tenant() AND c=finance.scope_company() AND a=finance.scope_actor()
 AND o.tenant_id=t AND o.company_id=c AND o.id=j
 AND EXISTS(SELECT 1 FROM finance.audit_event e WHERE e.tenant_id=t AND e.company_id=c
  AND e.command_id=o.command_id AND e.resource_id=o.resource_id AND e.actor_id=a)
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c
  AND m.actor_id=a AND m.active AND m.valid_from<=statement_timestamp()
  AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
$$;
ALTER FUNCTION finance.runtime_job_status(uuid,uuid,uuid,uuid) OWNER TO finance_job_reader;
REVOKE ALL ON FUNCTION finance.runtime_job_status(uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_job_status(uuid,uuid,uuid,uuid) TO finance_runtime;
