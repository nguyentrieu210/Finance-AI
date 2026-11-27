-- E18 isolated synthetic draft creation. No processing, sealing, approval or payment.
CREATE TABLE finance.dossier (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 owner_id uuid NOT NULL, code text NOT NULL,
 kind text NOT NULL CHECK(kind IN ('PAYMENT','ADVANCE','SETTLEMENT')),
 status text NOT NULL DEFAULT 'DRAFT' CHECK(status='DRAFT'),
 version bigint NOT NULL DEFAULT 0 CHECK(version=0),
 revision bigint NOT NULL DEFAULT 0 CHECK(revision=0),
 updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id), UNIQUE(tenant_id,code),
 UNIQUE(tenant_id,company_id,id,owner_id),
 FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id),
 FOREIGN KEY(tenant_id,owner_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(id<>'00000000-0000-0000-0000-000000000000')
);
CREATE TABLE finance.dossier_revision (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, dossier_id uuid NOT NULL,
 id uuid NOT NULL, owner_id uuid NOT NULL, number bigint NOT NULL CHECK(number=0),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id), UNIQUE(tenant_id,company_id,dossier_id,number),
 FOREIGN KEY(tenant_id,company_id,dossier_id,owner_id) REFERENCES finance.dossier(tenant_id,company_id,id,owner_id)
);
ALTER TABLE finance.dossier ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.dossier FORCE ROW LEVEL SECURITY;
ALTER TABLE finance.dossier_revision ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.dossier_revision FORCE ROW LEVEL SECURITY;
CREATE POLICY dossier_owner_read ON finance.dossier FOR SELECT TO finance_runtime
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND owner_id=finance.scope_actor());
CREATE POLICY dossier_owner_insert ON finance.dossier FOR INSERT TO finance_runtime
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND owner_id=finance.scope_actor());
CREATE POLICY revision_owner_read ON finance.dossier_revision FOR SELECT TO finance_runtime
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND owner_id=finance.scope_actor());
CREATE POLICY revision_owner_insert ON finance.dossier_revision FOR INSERT TO finance_runtime
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND owner_id=finance.scope_actor());
GRANT SELECT,INSERT ON finance.dossier,finance.dossier_revision TO finance_runtime;
-- Preserve organization receipt FK, add a separate typed FK for dossier receipts.
ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_tenant_id_company_id_result_id_fkey;
ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN ('prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier'));
ALTER TABLE finance.idempotency_record ADD COLUMN organization_result_id uuid GENERATED ALWAYS AS
 (CASE WHEN operation_id IN ('prototype.createOrganizationUnit','prototype.updateOrganizationUnit') THEN result_id ELSE NULL END) STORED,
 ADD COLUMN dossier_result_id uuid GENERATED ALWAYS AS
 (CASE WHEN operation_id='prototype.createDossier' THEN result_id ELSE NULL END) STORED;
ALTER TABLE finance.idempotency_record ADD FOREIGN KEY(tenant_id,company_id,organization_result_id) REFERENCES finance.organization_unit(tenant_id,company_id,id),
 ADD FOREIGN KEY(tenant_id,company_id,dossier_result_id,actor_id) REFERENCES finance.dossier(tenant_id,company_id,id,owner_id);
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN ('ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN ('ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED'));
CREATE INDEX dossier_owner_list ON finance.dossier(tenant_id,company_id,owner_id,id);
-- No UPDATE/DELETE on dossier or revisions. No permission granted to worker.
