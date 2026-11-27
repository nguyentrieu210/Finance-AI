DO $$
DECLARE created bigint;
BEGIN
 SELECT count(*) INTO created
 FROM finance.idempotency_record
 WHERE operation_id='prototype.createDossier';
 IF created<3
 OR (SELECT count(DISTINCT d.kind)
     FROM finance.idempotency_record i
     JOIN finance.dossier d
       ON d.tenant_id=i.tenant_id AND d.company_id=i.company_id AND d.id=i.result_id
     WHERE i.operation_id='prototype.createDossier')<3
 OR EXISTS(
    SELECT 1
    FROM finance.idempotency_record i
    LEFT JOIN finance.dossier d
      ON d.tenant_id=i.tenant_id AND d.company_id=i.company_id AND d.id=i.result_id
    WHERE i.operation_id='prototype.createDossier'
      AND (d.id IS NULL OR d.status<>'DRAFT')
 )
 OR EXISTS(
    SELECT 1
    FROM finance.idempotency_record i
    LEFT JOIN finance.dossier_revision r
      ON r.tenant_id=i.tenant_id AND r.company_id=i.company_id
     AND r.dossier_id=i.result_id AND r.number=0
    WHERE i.operation_id='prototype.createDossier' AND r.id IS NULL
 )
 OR EXISTS(
    SELECT 1
    FROM finance.idempotency_record i
    WHERE i.operation_id='prototype.createDossier'
      AND NOT EXISTS(
        SELECT 1 FROM finance.audit_event a
        WHERE a.tenant_id=i.tenant_id AND a.company_id=i.company_id
          AND a.resource_id=i.result_id AND a.action='DOSSIER_CREATED'
      )
 )
 OR EXISTS(
    SELECT 1
    FROM finance.idempotency_record i
    WHERE i.operation_id='prototype.createDossier'
      AND NOT EXISTS(
        SELECT 1 FROM finance.outbox o
        WHERE o.tenant_id=i.tenant_id AND o.company_id=i.company_id
          AND o.resource_id=i.result_id AND o.event_type='DOSSIER_CREATED'
      )
 )
 OR has_column_privilege('finance_runtime','finance.dossier','status','UPDATE')
 THEN
  RAISE EXCEPTION 'draft creation receipt/revision/audit/outbox or privilege invariant failed';
 END IF;

 IF (SELECT status FROM finance.dossier
     WHERE id='00000000-0000-0000-0000-000000001800')<>'EXPORTED'
 OR NOT EXISTS(
    SELECT 1
    FROM finance.package p
    JOIN finance.dossier_snapshot s
      ON (s.tenant_id,s.company_id,s.id)=(p.tenant_id,p.company_id,p.snapshot_id)
    WHERE s.dossier_id='00000000-0000-0000-0000-000000001800'
      AND p.state='READY'
      AND p.manifest_hash IS NOT NULL
      AND p.artifact_count=3
      AND p.artifact_bytes=360
 )
 THEN
  RAISE EXCEPTION 'fenced export worker did not persist verified package and EXPORTED transition';
 END IF;
END $$;
SELECT 'PASS: idempotent three-kind draft creation plus fenced verified export';

DO $$
DECLARE document_commands bigint;
BEGIN
 SELECT count(*) INTO document_commands
 FROM finance.idempotency_record
 WHERE operation_id='prototype.createDocument';
 IF document_commands<>5
 OR EXISTS(
    SELECT 1
    FROM finance.idempotency_record i
    LEFT JOIN finance.upload_intent u
      ON u.tenant_id=i.tenant_id AND u.company_id=i.company_id AND u.id=i.result_id
    WHERE i.operation_id='prototype.createDocument' AND u.id IS NULL
 )
 OR (SELECT sum(u.declared_bytes)
     FROM finance.idempotency_record i
     JOIN finance.upload_intent u
       ON u.tenant_id=i.tenant_id AND u.company_id=i.company_id AND u.id=i.result_id
     WHERE i.operation_id='prototype.createDocument')<>104857635
 OR EXISTS(
    SELECT 1
    FROM finance.idempotency_record i
    WHERE i.operation_id='prototype.createDocument'
      AND NOT EXISTS(
        SELECT 1 FROM finance.audit_event a
        WHERE a.tenant_id=i.tenant_id AND a.company_id=i.company_id
          AND a.resource_id=i.result_id AND a.action='DOCUMENT_INTENT_CREATED'
      )
 )
 OR EXISTS(
    SELECT 1
    FROM finance.idempotency_record i
    WHERE i.operation_id='prototype.createDocument'
      AND NOT EXISTS(
        SELECT 1 FROM finance.outbox o
        WHERE o.tenant_id=i.tenant_id AND o.company_id=i.company_id
          AND o.resource_id=i.result_id AND o.event_type='DOCUMENT_INTENT_CREATED'
      )
 )
 OR (SELECT sum(d.version)
     FROM finance.dossier d
     WHERE EXISTS(
       SELECT 1
       FROM finance.idempotency_record i
       JOIN finance.upload_intent u
         ON u.tenant_id=i.tenant_id AND u.company_id=i.company_id AND u.id=i.result_id
       WHERE i.operation_id='prototype.createDocument'
         AND u.tenant_id=d.tenant_id AND u.company_id=d.company_id AND u.dossier_id=d.id
     ))<>5
 OR has_table_privilege('finance_runtime','finance.upload_grant','SELECT')
 THEN
  RAISE EXCEPTION 'document intent receipt/version/audit/outbox invariant failed';
 END IF;
END $$;
SELECT 'PASS: document metadata authorization, quota, version replay and audit/outbox';

-- Regression: approver sees only case-related parents, not arbitrary dossiers.
BEGIN;
SET LOCAL ROLE finance_runtime;
SELECT set_config('finance.tenant_id','00000000-0000-0000-0000-000000000001',true),
 set_config('finance.company_id','00000000-0000-0000-0000-00000000000a',true),
 set_config('finance.actor_id','00000000-0000-0000-0000-0000000003f1',true);
DO $$
BEGIN
 IF NOT EXISTS(SELECT 1 FROM finance.dossier WHERE id='00000000-0000-0000-0000-000000002120')
 OR EXISTS(SELECT 1 FROM finance.dossier WHERE id='00000000-0000-0000-0000-000000001800')
 OR NOT EXISTS(SELECT 1 FROM finance.handoff WHERE id='00000000-0000-0000-0000-000000002315')
 OR EXISTS(SELECT 1 FROM finance.handoff WHERE external_ref='MANUAL-RECEIPT-1')
 THEN RAISE EXCEPTION 'case-parent visibility failed'; END IF;
END $$;
ROLLBACK;
SELECT 'PASS: case-bound approver parent visibility';
