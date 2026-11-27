-- Close the v2 registry inventory. These tables are private schema foundations:
-- no runtime grants are added here and no business state is implied.
CREATE TABLE finance.permission (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,actor_id uuid NOT NULL,
 action text NOT NULL CHECK(octet_length(action) BETWEEN 1 AND 64),
 resource_kind text NOT NULL CHECK(octet_length(resource_kind) BETWEEN 1 AND 64),
 active boolean NOT NULL DEFAULT true,valid_from timestamptz NOT NULL DEFAULT clock_timestamp(),valid_until timestamptz,
 PRIMARY KEY(tenant_id,company_id,id),FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(valid_until IS NULL OR valid_until>valid_from)
);
CREATE TABLE finance.delegation (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,grantor_id uuid NOT NULL,delegate_id uuid NOT NULL,
 action text NOT NULL CHECK(octet_length(action) BETWEEN 1 AND 64),currency text,amount_limit numeric(28,8),
 valid_from timestamptz NOT NULL,valid_until timestamptz NOT NULL,revoked boolean NOT NULL DEFAULT false,
 PRIMARY KEY(tenant_id,company_id,id),FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id),
 FOREIGN KEY(tenant_id,grantor_id) REFERENCES finance.identity_subject(tenant_id,id),
 FOREIGN KEY(tenant_id,delegate_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(grantor_id<>delegate_id AND valid_until>valid_from),
 CHECK((currency IS NULL AND amount_limit IS NULL) OR (currency ~ '^[A-Z]{3}$' AND amount_limit>=0))
);
CREATE TABLE finance.document_layout (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,document_id uuid NOT NULL,revision_id uuid NOT NULL,
 model_digest bytea NOT NULL CHECK(octet_length(model_digest)=32),page_count integer NOT NULL CHECK(page_count BETWEEN 1 AND 200),
 payload_digest bytea NOT NULL CHECK(octet_length(payload_digest)=32),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),FOREIGN KEY(tenant_id,company_id,document_id) REFERENCES finance.document_ref(tenant_id,company_id,id)
);
CREATE TABLE finance.match_result (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,dossier_id uuid NOT NULL,revision_id uuid NOT NULL,
 left_evidence_id uuid NOT NULL,right_evidence_id uuid NOT NULL,
 state text NOT NULL CHECK(state IN ('EXACT','SUGGESTED','AMBIGUOUS','UNMATCHED')),
 reason text NOT NULL CHECK(octet_length(reason) BETWEEN 1 AND 2048),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),FOREIGN KEY(tenant_id,company_id,dossier_id) REFERENCES finance.dossier(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,revision_id) REFERENCES finance.dossier_revision(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,left_evidence_id) REFERENCES finance.evidence(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,right_evidence_id) REFERENCES finance.evidence(tenant_id,company_id,id),
 CHECK(left_evidence_id<>right_evidence_id)
);
CREATE TABLE finance.match_decision (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,result_id uuid NOT NULL,actor_id uuid NOT NULL,
 decision text NOT NULL CHECK(decision IN ('CONFIRMED','REJECTED')),reason text NOT NULL CHECK(octet_length(reason) BETWEEN 1 AND 2048),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(tenant_id,company_id,id),
 UNIQUE(tenant_id,company_id,result_id),FOREIGN KEY(tenant_id,company_id,result_id) REFERENCES finance.match_result(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id)
);
CREATE TABLE finance.contract_line (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,snapshot_id uuid NOT NULL,contract_id uuid NOT NULL,id uuid NOT NULL,
 item_key text NOT NULL CHECK(octet_length(item_key) BETWEEN 1 AND 128),uom text NOT NULL CHECK(octet_length(uom) BETWEEN 1 AND 32),
 quantity numeric(28,8) NOT NULL CHECK(quantity>0),value numeric(28,8) NOT NULL CHECK(value>=0),
 currency text NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),PRIMARY KEY(tenant_id,company_id,snapshot_id,contract_id,id),
 FOREIGN KEY(tenant_id,company_id,snapshot_id,contract_id) REFERENCES finance.contract_ref(tenant_id,company_id,snapshot_id,id)
);
CREATE TABLE finance.invoice_line (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,snapshot_id uuid NOT NULL,invoice_id uuid NOT NULL,id uuid NOT NULL,
 item_key text NOT NULL CHECK(octet_length(item_key) BETWEEN 1 AND 128),uom text NOT NULL CHECK(octet_length(uom) BETWEEN 1 AND 32),
 quantity numeric(28,8) NOT NULL CHECK(quantity>0),value numeric(28,8) NOT NULL CHECK(value>=0),
 currency text NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),PRIMARY KEY(tenant_id,company_id,snapshot_id,invoice_id,id),
 FOREIGN KEY(tenant_id,company_id,snapshot_id,invoice_id) REFERENCES finance.invoice_ref(tenant_id,company_id,snapshot_id,id)
);
CREATE TABLE finance.settlement_link (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,snapshot_id uuid NOT NULL,id uuid NOT NULL,advance_id uuid NOT NULL,
 expense_ref text NOT NULL CHECK(octet_length(expense_ref) BETWEEN 1 AND 128),amount numeric(28,8) NOT NULL CHECK(amount>0),
 currency text NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,snapshot_id,id),
 FOREIGN KEY(tenant_id,company_id,snapshot_id,advance_id) REFERENCES finance.advance_ref(tenant_id,company_id,snapshot_id,id)
);
CREATE TABLE finance.rule_version (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,policy_id uuid NOT NULL,clause_id uuid NOT NULL,
 rule_key text NOT NULL CHECK(rule_key ~ '^[A-Za-z0-9._-]{1,64}$'),version bigint NOT NULL CHECK(version BETWEEN 1 AND 9007199254740991),
 severity text NOT NULL CHECK(severity IN ('BLOCK','WARNING','INFO')),waivable boolean NOT NULL,
 state text NOT NULL DEFAULT 'DRAFT' CHECK(state IN ('DRAFT','PUBLISHED','RETIRED','REVOKED')),
 definition jsonb NOT NULL CHECK(jsonb_typeof(definition)='object'),author_id uuid NOT NULL,checker_id uuid,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),published_at timestamptz,
 PRIMARY KEY(tenant_id,company_id,id),UNIQUE(tenant_id,company_id,rule_key,version),
 FOREIGN KEY(tenant_id,company_id,policy_id,clause_id) REFERENCES finance.policy_clause(tenant_id,company_id,policy_id,id),
 FOREIGN KEY(tenant_id,author_id) REFERENCES finance.identity_subject(tenant_id,id),
 FOREIGN KEY(tenant_id,checker_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(checker_id IS NULL OR checker_id<>author_id)
);
CREATE TABLE finance.rule_simulation (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,rule_id uuid NOT NULL,actor_id uuid NOT NULL,
 corpus_digest bytea NOT NULL CHECK(octet_length(corpus_digest)=32),result jsonb NOT NULL CHECK(jsonb_typeof(result)='object'),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,rule_id) REFERENCES finance.rule_version(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id)
);
CREATE TABLE finance.check_acknowledgment (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,run_id uuid NOT NULL,rule_key text NOT NULL,actor_id uuid NOT NULL,
 reason text NOT NULL CHECK(octet_length(reason) BETWEEN 1 AND 2048),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),UNIQUE(tenant_id,company_id,run_id,rule_key,actor_id),
 FOREIGN KEY(tenant_id,company_id,run_id,rule_key) REFERENCES finance.check_result(tenant_id,company_id,run_id,rule_key),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id)
);
CREATE TABLE finance.dossier_snapshot (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,dossier_id uuid NOT NULL,revision_id uuid NOT NULL,
 policy_id uuid,check_run_id uuid,input_hash bytea NOT NULL CHECK(octet_length(input_hash)=32),
 canonical_manifest_hash bytea NOT NULL CHECK(octet_length(canonical_manifest_hash)=32),sealed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),UNIQUE(tenant_id,company_id,dossier_id,revision_id,input_hash),
 FOREIGN KEY(tenant_id,company_id,dossier_id) REFERENCES finance.dossier(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,revision_id) REFERENCES finance.dossier_revision(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,policy_id) REFERENCES finance.policy_version(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,check_run_id) REFERENCES finance.check_run(tenant_id,company_id,id)
);
CREATE TABLE finance.snapshot_member (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,snapshot_id uuid NOT NULL,id uuid NOT NULL,
 kind text NOT NULL CHECK(kind ~ '^[A-Z_]{1,64}$'),resource_id uuid NOT NULL,digest bytea NOT NULL CHECK(octet_length(digest)=32),
 PRIMARY KEY(tenant_id,company_id,snapshot_id,id),
 FOREIGN KEY(tenant_id,company_id,snapshot_id) REFERENCES finance.dossier_snapshot(tenant_id,company_id,id)
);
CREATE TABLE finance.workflow_version (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,policy_id uuid,name text NOT NULL CHECK(octet_length(name) BETWEEN 1 AND 200),
 version bigint NOT NULL CHECK(version BETWEEN 1 AND 9007199254740991),state text NOT NULL CHECK(state IN ('DRAFT','PUBLISHED','RETIRED','REVOKED')),
 definition jsonb NOT NULL CHECK(jsonb_typeof(definition)='object'),author_id uuid NOT NULL,checker_id uuid,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),published_at timestamptz,
 PRIMARY KEY(tenant_id,company_id,id),UNIQUE(tenant_id,company_id,name,version),
 FOREIGN KEY(tenant_id,company_id,policy_id) REFERENCES finance.policy_version(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,author_id) REFERENCES finance.identity_subject(tenant_id,id),
 FOREIGN KEY(tenant_id,checker_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(checker_id IS NULL OR checker_id<>author_id)
);
CREATE TABLE finance.review_decision (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,snapshot_id uuid NOT NULL,actor_id uuid NOT NULL,
 decision text NOT NULL CHECK(decision IN ('ACCEPTED','RETURNED')),reason text NOT NULL CHECK(octet_length(reason) BETWEEN 1 AND 4096),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,snapshot_id) REFERENCES finance.dossier_snapshot(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id)
);
CREATE TABLE finance.critical_confirmation (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,snapshot_id uuid NOT NULL,field_path text NOT NULL,actor_id uuid NOT NULL,
 evidence_id uuid NOT NULL,decision_version bigint NOT NULL CHECK(decision_version BETWEEN 1 AND 9007199254740991),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(tenant_id,company_id,snapshot_id,field_path,actor_id),
 FOREIGN KEY(tenant_id,company_id,snapshot_id) REFERENCES finance.dossier_snapshot(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,evidence_id) REFERENCES finance.evidence(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id)
);
CREATE TABLE finance.form_version (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,policy_id uuid,name text NOT NULL CHECK(octet_length(name) BETWEEN 1 AND 200),
 version bigint NOT NULL CHECK(version BETWEEN 1 AND 9007199254740991),state text NOT NULL CHECK(state IN ('DRAFT','PUBLISHED','RETIRED','REVOKED')),
 schema_json jsonb NOT NULL CHECK(jsonb_typeof(schema_json)='object'),author_id uuid NOT NULL,checker_id uuid,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),published_at timestamptz,
 PRIMARY KEY(tenant_id,company_id,id),UNIQUE(tenant_id,company_id,name,version),
 FOREIGN KEY(tenant_id,company_id,policy_id) REFERENCES finance.policy_version(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,author_id) REFERENCES finance.identity_subject(tenant_id,id),
 FOREIGN KEY(tenant_id,checker_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(checker_id IS NULL OR checker_id<>author_id)
);
CREATE TABLE finance.template_version (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,policy_id uuid,name text NOT NULL CHECK(octet_length(name) BETWEEN 1 AND 200),
 version bigint NOT NULL CHECK(version BETWEEN 1 AND 9007199254740991),state text NOT NULL CHECK(state IN ('DRAFT','PUBLISHED','RETIRED','REVOKED')),
 format text NOT NULL CHECK(format IN ('DOCX','XLSX','PDF')),source_digest bytea NOT NULL CHECK(octet_length(source_digest)=32),
 author_id uuid NOT NULL,checker_id uuid,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),published_at timestamptz,
 PRIMARY KEY(tenant_id,company_id,id),UNIQUE(tenant_id,company_id,name,version),
 FOREIGN KEY(tenant_id,company_id,policy_id) REFERENCES finance.policy_version(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,author_id) REFERENCES finance.identity_subject(tenant_id,id),
 FOREIGN KEY(tenant_id,checker_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(checker_id IS NULL OR checker_id<>author_id)
);
CREATE TABLE finance.package (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,snapshot_id uuid NOT NULL,template_id uuid NOT NULL,
 state text NOT NULL CHECK(state IN ('QUEUED','READY','FAILED')),manifest_hash bytea,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),UNIQUE(tenant_id,company_id,snapshot_id,template_id),
 FOREIGN KEY(tenant_id,company_id,snapshot_id) REFERENCES finance.dossier_snapshot(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,template_id) REFERENCES finance.template_version(tenant_id,company_id,id),
 CHECK((state='READY')=(manifest_hash IS NOT NULL)),CHECK(manifest_hash IS NULL OR octet_length(manifest_hash)=32)
);
CREATE TABLE finance.waiver (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,run_id uuid NOT NULL,rule_key text NOT NULL,dossier_id uuid NOT NULL,
 revision_id uuid NOT NULL,input_hash bytea NOT NULL CHECK(octet_length(input_hash)=32),requester_id uuid NOT NULL,approver_id uuid,
 reason text NOT NULL CHECK(octet_length(reason) BETWEEN 1 AND 4096),expires_at timestamptz NOT NULL,
 state text NOT NULL DEFAULT 'REQUESTED' CHECK(state IN ('REQUESTED','APPROVED','REJECTED','EXPIRED','REVOKED')),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,run_id,rule_key) REFERENCES finance.check_result(tenant_id,company_id,run_id,rule_key),
 FOREIGN KEY(tenant_id,company_id,dossier_id) REFERENCES finance.dossier(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,revision_id) REFERENCES finance.dossier_revision(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,requester_id) REFERENCES finance.identity_subject(tenant_id,id),
 FOREIGN KEY(tenant_id,approver_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(approver_id IS NULL OR approver_id<>requester_id)
);
CREATE TABLE finance.risk_signal (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,snapshot_id uuid NOT NULL,
 code text NOT NULL CHECK(code ~ '^[A-Z0-9_]{1,64}$'),severity text NOT NULL CHECK(severity IN ('BLOCK','WARNING','INFO')),
 evidence_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[],created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),FOREIGN KEY(tenant_id,company_id,snapshot_id) REFERENCES finance.dossier_snapshot(tenant_id,company_id,id)
);
CREATE TABLE finance.risk_decision (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,signal_id uuid NOT NULL,actor_id uuid NOT NULL,
 resolution text NOT NULL CHECK(resolution IN ('CONFIRMED','DISMISSED','ESCALATED')),
 reason text NOT NULL CHECK(octet_length(reason) BETWEEN 1 AND 4096),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),UNIQUE(tenant_id,company_id,signal_id),
 FOREIGN KEY(tenant_id,company_id,signal_id) REFERENCES finance.risk_signal(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id)
);
CREATE TABLE finance.audit_checkpoint (
 tenant_id uuid NOT NULL,id uuid NOT NULL,sequence bigint NOT NULL CHECK(sequence>0),digest bytea NOT NULL CHECK(octet_length(digest)=32),
 anchor_ref text NOT NULL CHECK(octet_length(anchor_ref) BETWEEN 1 AND 512),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),verified_by uuid,
 PRIMARY KEY(tenant_id,id),UNIQUE(tenant_id,sequence,digest),FOREIGN KEY(tenant_id) REFERENCES finance.tenant(id),
 FOREIGN KEY(tenant_id,verified_by) REFERENCES finance.identity_subject(tenant_id,id)
);
CREATE TABLE finance.payment_ref (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,snapshot_id uuid NOT NULL,id uuid NOT NULL,
 external_key text NOT NULL CHECK(octet_length(external_key) BETWEEN 1 AND 128),currency text NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),
 amount numeric(28,8) NOT NULL CHECK(amount>0),state text NOT NULL CHECK(state IN ('UNPAID','PARTIALLY_PAID','PAID','PARTIALLY_REVERSED','REVERSED')),
 source_sequence bigint,occurred_at timestamptz NOT NULL,PRIMARY KEY(tenant_id,company_id,snapshot_id,id),
 UNIQUE(tenant_id,company_id,snapshot_id,external_key),
 FOREIGN KEY(tenant_id,company_id,snapshot_id) REFERENCES finance.reference_snapshot(tenant_id,company_id,id)
);
CREATE TABLE finance.handoff (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,snapshot_id uuid NOT NULL,package_id uuid NOT NULL,
 channel text NOT NULL CHECK(channel IN ('MANUAL','DO_API')),state text NOT NULL CHECK(state IN ('NOT_SUBMITTED','DISPATCH_PENDING','ACKNOWLEDGED','APPROVED','REJECTED','UNKNOWN')),
 external_ref text,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(tenant_id,company_id,id),
 UNIQUE(tenant_id,company_id,package_id),FOREIGN KEY(tenant_id,company_id,snapshot_id) REFERENCES finance.dossier_snapshot(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,package_id) REFERENCES finance.package(tenant_id,company_id,id)
);
CREATE TABLE finance.external_status (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,handoff_id uuid NOT NULL,sequence bigint NOT NULL CHECK(sequence>0),
 state text NOT NULL CHECK(state IN ('ACKNOWLEDGED','APPROVED','REJECTED','UNKNOWN')),source_digest bytea NOT NULL CHECK(octet_length(source_digest)=32),
 independently_verified boolean NOT NULL,observed_at timestamptz NOT NULL,PRIMARY KEY(tenant_id,company_id,id),
 UNIQUE(tenant_id,company_id,handoff_id,sequence),FOREIGN KEY(tenant_id,company_id,handoff_id) REFERENCES finance.handoff(tenant_id,company_id,id)
);
CREATE TABLE finance.external_attempt (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,handoff_id uuid NOT NULL,attempt integer NOT NULL CHECK(attempt BETWEEN 1 AND 100),
 request_digest bytea NOT NULL CHECK(octet_length(request_digest)=32),state text NOT NULL CHECK(state IN ('PENDING','SENT','TIMEOUT','FAILED','ACKNOWLEDGED')),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),PRIMARY KEY(tenant_id,company_id,id),
 UNIQUE(tenant_id,company_id,handoff_id,attempt),FOREIGN KEY(tenant_id,company_id,handoff_id) REFERENCES finance.handoff(tenant_id,company_id,id)
);
CREATE TABLE finance.reconciliation_case (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,handoff_id uuid NOT NULL,
 state text NOT NULL CHECK(state IN ('OPEN','RESOLVED','ESCALATED')),reason text NOT NULL CHECK(octet_length(reason) BETWEEN 1 AND 4096),
 assigned_actor uuid,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),resolved_at timestamptz,
 PRIMARY KEY(tenant_id,company_id,id),FOREIGN KEY(tenant_id,company_id,handoff_id) REFERENCES finance.handoff(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,assigned_actor) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK((state='RESOLVED')=(resolved_at IS NOT NULL))
);

DO $$
DECLARE n text;
BEGIN
 FOREACH n IN ARRAY ARRAY[
 'permission','delegation','document_layout','match_result','match_decision','contract_line','invoice_line','settlement_link',
 'rule_version','rule_simulation','check_acknowledgment','dossier_snapshot','snapshot_member','workflow_version','review_decision',
 'critical_confirmation','form_version','template_version','package','waiver','risk_signal','risk_decision','audit_checkpoint',
 'payment_ref','handoff','external_status','external_attempt','reconciliation_case'
 ] LOOP
  EXECUTE format('ALTER TABLE finance.%I ENABLE ROW LEVEL SECURITY',n);
  EXECUTE format('ALTER TABLE finance.%I FORCE ROW LEVEL SECURITY',n);
 END LOOP;
END $$;

-- Immutable evidence/effect entities cannot be edited even by accidental future grants.
CREATE TRIGGER document_layout_immutable BEFORE UPDATE OR DELETE ON finance.document_layout FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER match_result_immutable BEFORE UPDATE OR DELETE ON finance.match_result FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER match_decision_immutable BEFORE UPDATE OR DELETE ON finance.match_decision FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER contract_line_immutable BEFORE UPDATE OR DELETE ON finance.contract_line FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER invoice_line_immutable BEFORE UPDATE OR DELETE ON finance.invoice_line FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER settlement_link_immutable BEFORE UPDATE OR DELETE ON finance.settlement_link FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER rule_simulation_immutable BEFORE UPDATE OR DELETE ON finance.rule_simulation FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER check_ack_immutable BEFORE UPDATE OR DELETE ON finance.check_acknowledgment FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER dossier_snapshot_immutable BEFORE UPDATE OR DELETE ON finance.dossier_snapshot FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER snapshot_member_immutable BEFORE UPDATE OR DELETE ON finance.snapshot_member FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER review_decision_immutable BEFORE UPDATE OR DELETE ON finance.review_decision FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER critical_confirmation_immutable BEFORE UPDATE OR DELETE ON finance.critical_confirmation FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER risk_signal_immutable BEFORE UPDATE OR DELETE ON finance.risk_signal FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER risk_decision_immutable BEFORE UPDATE OR DELETE ON finance.risk_decision FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER audit_checkpoint_immutable BEFORE UPDATE OR DELETE ON finance.audit_checkpoint FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER payment_ref_immutable BEFORE UPDATE OR DELETE ON finance.payment_ref FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER external_status_immutable BEFORE UPDATE OR DELETE ON finance.external_status FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER external_attempt_immutable BEFORE UPDATE OR DELETE ON finance.external_attempt FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
