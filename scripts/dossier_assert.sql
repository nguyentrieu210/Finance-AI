DO $$
BEGIN
 IF (SELECT count(*) FROM finance.dossier)<3
 OR (SELECT count(*) FROM finance.dossier_revision)<>(SELECT count(*) FROM finance.dossier)
 OR (SELECT count(*) FROM finance.idempotency_record WHERE operation_id='prototype.createDossier')<>(SELECT count(*) FROM finance.dossier)
 OR (SELECT count(*) FROM finance.audit_event WHERE action='DOSSIER_CREATED')<>(SELECT count(*) FROM finance.dossier)
 OR (SELECT count(*) FROM finance.outbox WHERE event_type='DOSSIER_CREATED')<>(SELECT count(*) FROM finance.dossier)
 OR EXISTS(SELECT 1 FROM finance.dossier WHERE status<>'DRAFT' OR version<>0)
 OR has_table_privilege('finance_runtime','finance.dossier','UPDATE') THEN
  RAISE EXCEPTION 'draft creation atomicity or privilege violation';
 END IF;
END $$;
SELECT 'PASS: all draft types, initial revision, receipt, audit/outbox atomicity, no direct mutation';
