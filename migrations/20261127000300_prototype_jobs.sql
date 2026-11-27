-- Synthetic-only worker, separate from runtime and migration role.
CREATE ROLE finance_job_worker NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_job_worker;
CREATE TABLE finance.worker_scope (
 login_name name NOT NULL, tenant_id uuid NOT NULL, company_id uuid NOT NULL,
 active boolean NOT NULL DEFAULT true,
 PRIMARY KEY(login_name,tenant_id,company_id),
 FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id)
);
ALTER TABLE finance.worker_scope ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.worker_scope FORCE ROW LEVEL SECURITY;
CREATE POLICY worker_assignment_read ON finance.worker_scope FOR SELECT TO finance_job_worker USING(login_name=session_user AND active);
GRANT SELECT ON finance.worker_scope TO finance_job_worker;
CREATE FUNCTION finance.worker_scope_allowed(t uuid,c uuid) RETURNS boolean LANGUAGE sql STABLE
SET search_path=pg_catalog AS $$
 SELECT t=finance.scope_tenant() AND c=finance.scope_company() AND EXISTS(
 SELECT 1 FROM finance.worker_scope w WHERE w.login_name=session_user AND w.tenant_id=t AND w.company_id=c AND w.active)
$$;
REVOKE ALL ON FUNCTION finance.worker_scope_allowed(uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.worker_scope_allowed(uuid,uuid) TO finance_job_worker;
CREATE POLICY outbox_worker_read ON finance.outbox FOR SELECT TO finance_job_worker USING(finance.worker_scope_allowed(tenant_id,company_id));
GRANT SELECT ON finance.outbox TO finance_job_worker;
CREATE TABLE finance.job (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 state text NOT NULL DEFAULT 'QUEUED' CHECK(state IN ('QUEUED','LEASED','RETRY','SUCCEEDED','DEAD')),
 attempts integer NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 3),
 fence bigint NOT NULL DEFAULT 0 CHECK(fence>=0),
 available_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 lease_until timestamptz, leased_by uuid,
 last_error text CHECK(last_error IN ('TRANSIENT','PERMANENT','LEASE_EXPIRED')),
 PRIMARY KEY(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,id) REFERENCES finance.outbox(tenant_id,company_id,id),
 CHECK((state='LEASED')=(lease_until IS NOT NULL AND leased_by IS NOT NULL)),
 CHECK((lease_until IS NULL)=(leased_by IS NULL)),
 CHECK(fence=attempts)
);
CREATE INDEX job_ready ON finance.job(tenant_id,company_id,available_at,id) WHERE state IN ('QUEUED','RETRY','LEASED');
CREATE TABLE finance.job_attempt (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, job_id uuid NOT NULL,
 fence bigint NOT NULL CHECK(fence BETWEEN 1 AND 3), worker_id uuid NOT NULL,
 state text NOT NULL CHECK(state IN ('ACTIVE','EXPIRED','RETRY','DEAD','SUCCEEDED')),
 started_at timestamptz NOT NULL DEFAULT clock_timestamp(), lease_until timestamptz NOT NULL,
 finished_at timestamptz,
 PRIMARY KEY(tenant_id,company_id,job_id,fence),
 FOREIGN KEY(tenant_id,company_id,job_id) REFERENCES finance.job(tenant_id,company_id,id),
 CHECK((state='ACTIVE')=(finished_at IS NULL))
);
CREATE TABLE finance.inbox (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, event_id uuid NOT NULL,
 fence bigint NOT NULL, handled_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,event_id),
 FOREIGN KEY(tenant_id,company_id,event_id,fence) REFERENCES finance.job_attempt(tenant_id,company_id,job_id,fence)
);
ALTER TABLE finance.job ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.job FORCE ROW LEVEL SECURITY;
CREATE POLICY job_scope ON finance.job TO finance_job_worker USING(finance.worker_scope_allowed(tenant_id,company_id)) WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
ALTER TABLE finance.job_attempt ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.job_attempt FORCE ROW LEVEL SECURITY;
CREATE POLICY attempt_scope ON finance.job_attempt TO finance_job_worker USING(finance.worker_scope_allowed(tenant_id,company_id)) WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
ALTER TABLE finance.inbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.inbox FORCE ROW LEVEL SECURITY;
CREATE POLICY inbox_scope ON finance.inbox TO finance_job_worker USING(finance.worker_scope_allowed(tenant_id,company_id)) WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
GRANT SELECT,INSERT ON finance.job,finance.job_attempt,finance.inbox TO finance_job_worker;
GRANT UPDATE(state,attempts,fence,available_at,lease_until,leased_by,last_error) ON finance.job TO finance_job_worker;
GRANT UPDATE(state,lease_until,finished_at) ON finance.job_attempt TO finance_job_worker;
-- No delete, inbox update, outbox mutation, financial or organization mutation for worker.
