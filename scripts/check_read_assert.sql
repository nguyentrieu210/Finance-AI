-- Run after actual OCR persistence and independent review in disposable PostgreSQL.
BEGIN;
SET LOCAL ROLE finance_runtime;
DO $$
DECLARE t text:='00000000-0000-0000-0000-000000000001';
 c text:='00000000-0000-0000-0000-00000000000a';
 d text:='00000000-0000-0000-0000-000000001f41';
 a text:='00000000-0000-0000-0000-0000000003e9';
BEGIN
 IF EXISTS(SELECT 1 FROM finance.check_run) OR EXISTS(SELECT 1 FROM finance.check_result)
 THEN RAISE EXCEPTION 'unscoped diagnostics visible'; END IF;
 PERFORM set_config('finance.tenant_id',t,true);
 PERFORM set_config('finance.company_id',c,true);
 PERFORM set_config('finance.actor_id',a,true);
 IF (SELECT count(*) FROM finance.check_run WHERE dossier_id=d::uuid)<>1 THEN
  RAISE EXCEPTION 'current dossier check_run count mismatch: %',
   (SELECT count(*) FROM finance.check_run WHERE dossier_id=d::uuid);
 END IF;
 IF EXISTS(
  SELECT 1 FROM finance.check_run r
  JOIN finance.dossier ds
   ON (ds.tenant_id,ds.company_id,ds.id)=(r.tenant_id,r.company_id,r.dossier_id)
  WHERE r.dossier_id=d::uuid AND r.dossier_version<>ds.version
 ) THEN
  RAISE EXCEPTION 'stale dossier check_run visible';
 END IF;
 IF (
  SELECT count(*) FROM finance.check_result
  WHERE run_id IN (
   SELECT id FROM finance.check_run WHERE dossier_id=d::uuid
  )
 )<5 THEN
  RAISE EXCEPTION 'current dossier check_result count too small: %',
   (SELECT count(*) FROM finance.check_result
    WHERE run_id IN (
     SELECT id FROM finance.check_run WHERE dossier_id=d::uuid
    ));
 END IF;
 IF NOT EXISTS(
  SELECT 1 FROM finance.check_result
  WHERE rule_key='scan.release' AND outcome='UNKNOWN'
   AND run_id IN (
    SELECT id FROM finance.check_run WHERE dossier_id=d::uuid
   )
 ) THEN
  RAISE EXCEPTION 'current dossier scan.release UNKNOWN diagnostic missing';
 END IF;
 IF has_table_privilege('finance_runtime','finance.check_result','INSERT,UPDATE,DELETE')
 OR has_table_privilege('finance_runtime','finance.check_run','INSERT,UPDATE,DELETE')
 THEN RAISE EXCEPTION 'diagnostic write grant'; END IF;
 PERFORM set_config('finance.actor_id','00000000-0000-0000-0000-0000000003e8',true);
 IF EXISTS(SELECT 1 FROM finance.check_result) THEN RAISE EXCEPTION 'other actor sees diagnostics'; END IF;
 PERFORM set_config('finance.actor_id',a,true);
 PERFORM set_config('finance.company_id','00000000-0000-0000-0000-00000000000b',true);
 IF EXISTS(SELECT 1 FROM finance.check_result) THEN RAISE EXCEPTION 'cross-company diagnostics visible'; END IF;
END $$;
ROLLBACK;
SELECT 'PASS: runtime current owner preflight diagnostics, scope isolation and no write grant';
