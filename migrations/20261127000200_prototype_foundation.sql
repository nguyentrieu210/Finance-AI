-- G02 synthetic-only prototype, migration owner applies. No runtime login credentials.
CREATE ROLE finance_runtime NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_runtime;

CREATE FUNCTION finance.scope_tenant() RETURNS uuid LANGUAGE sql STABLE
SET search_path = pg_catalog AS $$ SELECT NULLIF(current_setting('finance.tenant_id',true),'')::uuid $$;
CREATE FUNCTION finance.scope_company() RETURNS uuid LANGUAGE sql STABLE
SET search_path = pg_catalog AS $$ SELECT NULLIF(current_setting('finance.company_id',true),'')::uuid $$;
CREATE FUNCTION finance.scope_actor() RETURNS uuid LANGUAGE sql STABLE
SET search_path = pg_catalog AS $$ SELECT NULLIF(current_setting('finance.actor_id',true),'')::uuid $$;

CREATE TABLE finance.tenant (
 id uuid PRIMARY KEY CHECK(id <> '00000000-0000-0000-0000-000000000000'),
 code text NOT NULL UNIQUE CHECK(length(code) BETWEEN 1 AND 64), active boolean NOT NULL DEFAULT true
);
CREATE TABLE finance.company (
 tenant_id uuid NOT NULL REFERENCES finance.tenant(id),
 id uuid NOT NULL, code text NOT NULL CHECK(length(code) BETWEEN 1 AND 64),
 name text NOT NULL CHECK(length(name) BETWEEN 1 AND 200), active boolean NOT NULL DEFAULT true,
 PRIMARY KEY(tenant_id,id), UNIQUE(tenant_id,code)
);
CREATE TABLE finance.identity_subject (
 tenant_id uuid NOT NULL REFERENCES finance.tenant(id), id uuid NOT NULL,
 issuer text NOT NULL CHECK(length(issuer)>0), subject text NOT NULL CHECK(length(subject)>0),
 active boolean NOT NULL DEFAULT true,
 PRIMARY KEY(tenant_id,id), UNIQUE(tenant_id,issuer,subject)
);
CREATE TABLE finance.membership (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, actor_id uuid NOT NULL,
 role text NOT NULL CHECK(role IN ('ORG_ADMIN','REQUESTER','ACCOUNTANT','AUDITOR','FINANCE_APPROVER','POLICY_ADMIN')),
 active boolean NOT NULL DEFAULT true,
 valid_from timestamptz NOT NULL DEFAULT now(), valid_until timestamptz,
 PRIMARY KEY(tenant_id,company_id,actor_id,role),
 FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(valid_until IS NULL OR valid_until>valid_from)
);
CREATE TABLE finance.organization_unit (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 parent_id uuid, code text NOT NULL CHECK(length(code) BETWEEN 1 AND 64),
 name text NOT NULL CHECK(length(name) BETWEEN 1 AND 200),
 kind text NOT NULL CHECK(kind IN ('DEPARTMENT','COST_CENTER')),
 active boolean NOT NULL DEFAULT true,
 PRIMARY KEY(tenant_id,company_id,id), UNIQUE(tenant_id,company_id,code),
 FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id),
 FOREIGN KEY(tenant_id,company_id,parent_id) REFERENCES finance.organization_unit(tenant_id,company_id,id),
 CHECK(parent_id IS NULL OR parent_id<>id)
);
CREATE TABLE finance.audit_event (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 actor_id uuid NOT NULL, command_id uuid NOT NULL, correlation_id uuid NOT NULL,
 resource_id uuid NOT NULL, action text NOT NULL CHECK(action IN ('ORG_UNIT_CREATED')),
 occurred_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),
 UNIQUE(tenant_id,company_id,command_id,action),
 FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id)
);
CREATE TABLE finance.outbox (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 command_id uuid NOT NULL, resource_id uuid NOT NULL,
 event_type text NOT NULL CHECK(event_type IN ('ORG_UNIT_CREATED')),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),
 UNIQUE(tenant_id,company_id,command_id,event_type),
 FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id)
);
CREATE TABLE finance.idempotency_record (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, actor_id uuid NOT NULL,
 operation_id text NOT NULL CHECK(operation_id='prototype.createOrganizationUnit'),
 key text NOT NULL CHECK(key ~ '^[A-Za-z0-9._~-]{1,128}$'),
 request_hash bytea NOT NULL CHECK(octet_length(request_hash)=32),
 result_id uuid NOT NULL, created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,actor_id,operation_id,key),
 FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id),
 FOREIGN KEY(tenant_id,company_id,result_id) REFERENCES finance.organization_unit(tenant_id,company_id,id)
);
-- Scope helpers are defense in depth. Custom GUCs are set by trusted server code, not a security boundary against arbitrary SQL.
ALTER TABLE finance.tenant ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.tenant FORCE ROW LEVEL SECURITY;
CREATE POLICY tenant_read ON finance.tenant FOR SELECT TO finance_runtime USING(id=finance.scope_tenant());
ALTER TABLE finance.company ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.company FORCE ROW LEVEL SECURITY;
CREATE POLICY company_read ON finance.company FOR SELECT TO finance_runtime USING(tenant_id=finance.scope_tenant() AND id=finance.scope_company());
ALTER TABLE finance.identity_subject ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.identity_subject FORCE ROW LEVEL SECURITY;
CREATE POLICY identity_read ON finance.identity_subject FOR SELECT TO finance_runtime USING(tenant_id=finance.scope_tenant() AND id=finance.scope_actor());
ALTER TABLE finance.membership ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.membership FORCE ROW LEVEL SECURITY;
CREATE POLICY membership_read ON finance.membership FOR SELECT TO finance_runtime USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
ALTER TABLE finance.organization_unit ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.organization_unit FORCE ROW LEVEL SECURITY;
CREATE POLICY unit_read ON finance.organization_unit FOR SELECT TO finance_runtime USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY unit_insert ON finance.organization_unit FOR INSERT TO finance_runtime WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
ALTER TABLE finance.audit_event ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.audit_event FORCE ROW LEVEL SECURITY;
CREATE POLICY audit_insert ON finance.audit_event FOR INSERT TO finance_runtime WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
-- No runtime audit SELECT: a separate scoped auditor reader is future work.
ALTER TABLE finance.outbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.outbox FORCE ROW LEVEL SECURITY;
CREATE POLICY outbox_insert ON finance.outbox FOR INSERT TO finance_runtime WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
ALTER TABLE finance.idempotency_record ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.idempotency_record FORCE ROW LEVEL SECURITY;
CREATE POLICY idem_read ON finance.idempotency_record FOR SELECT TO finance_runtime USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY idem_insert ON finance.idempotency_record FOR INSERT TO finance_runtime WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
GRANT SELECT ON finance.tenant,finance.company,finance.identity_subject,finance.membership,finance.organization_unit,finance.idempotency_record TO finance_runtime;
GRANT INSERT ON finance.organization_unit,finance.audit_event,finance.outbox,finance.idempotency_record TO finance_runtime;
-- No UPDATE/DELETE/TRUNCATE, no runtime membership management or payment tables.
