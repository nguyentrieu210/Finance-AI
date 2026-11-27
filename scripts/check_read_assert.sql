-- Run after actual OCR persistence and independent review in disposable PostgreSQL.
BEGIN;
SET LOCAL ROLE finance_runtime;
DO $$
DECLARE t text:='00000000-0000-0000-0000-000000000001';
 c text:='00000000-0000-0000-0000-00000000000a';
 a text:='00000000-0000-0000-0000-0000000003e9';
BEGIN
 IF EXISTS(SELECT 1 FROM finance.check_run) OR EXISTS(SELECT 1 FROM finance.check_result)
 THEN RAISE EXCEPTION 'unscoped diagnostics visible'; END IF;
 PERFORM set_config('finance.tenant_id',t,true);
 PERFORM set_config('finance.company_id',c,true);
 PERFORM set_config('finance.actor_id',a,true);
 IF (SELECT count(*) FROM finance.check_run)<>1
 OR (SELECT count(*) FROM finance.check_result)<5
 OR NOT EXISTS(SELECT 1 FROM finance.check_result WHERE rule_key='scan.release' AND outcome='UNKNOWN')
 THEN RAISE EXCEPTION 'current owner diagnostics missing or stale run exposed'; END IF;
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
