DO $$
BEGIN
 IF (SELECT count(*) FROM finance.dossier)<3
 OR (SELECT count(*) FROM finance.dossier_revision)<>(SELECT count(*) FROM finance.dossier)
 OR (SELECT count(*) FROM finance.idempotency_record WHERE operation_id='prototype.createDossier')<>(SELECT count(*) FROM finance.dossier)
 OR (SELECT count(*) FROM finance.audit_event WHERE action='DOSSIER_CREATED')<>(SELECT count(*) FROM finance.dossier)
 OR (SELECT count(*) FROM finance.outbox WHERE event_type='DOSSIER_CREATED')<>(SELECT count(*) FROM finance.dossier)
 OR EXISTS(SELECT 1 FROM finance.dossier WHERE status<>'DRAFT')
 OR has_column_privilege('finance_runtime','finance.dossier','status','UPDATE') THEN
  RAISE EXCEPTION 'draft creation atomicity or privilege violation';
 END IF;
 IF (SELECT status FROM finance.dossier WHERE id='00000000-0000-0000-0000-000000001800')<>'EXPORTED'
 OR NOT EXISTS(SELECT 1 FROM finance.package p JOIN finance.dossier_snapshot s ON s.id=p.snapshot_id
  WHERE s.dossier_id='00000000-0000-0000-0000-000000001800' AND p.state='READY'
  AND p.manifest_hash IS NOT NULL AND p.artifact_count=3 AND p.artifact_bytes=360)
 THEN RAISE EXCEPTION 'fenced export worker did not persist verified package and EXPORTED transition'; END IF;
END $;
SELECT 'PASS: all draft types, initial revision, receipt, audit/outbox atomicity, no direct mutation';

DO $$
BEGIN
 IF (SELECT count(*) FROM finance.upload_intent WHERE document_kind IS NOT NULL)<>5
 OR (SELECT sum(declared_bytes) FROM finance.upload_intent WHERE document_kind IS NOT NULL)<>104857635
 OR (SELECT count(*) FROM finance.idempotency_record WHERE operation_id='prototype.createDocument')<>5
 OR (SELECT count(*) FROM finance.audit_event WHERE action='DOCUMENT_INTENT_CREATED')<>5
 OR (SELECT count(*) FROM finance.outbox WHERE event_type='DOCUMENT_INTENT_CREATED')<>5
 OR (SELECT sum(version) FROM finance.dossier)<>5
 OR has_table_privilege('finance_runtime','finance.upload_grant','SELECT')
 THEN RAISE EXCEPTION 'document intent atomicity failed'; END IF;
END $$;
SELECT 'PASS: document metadata authorization, quota, version replay and audit/outbox';
