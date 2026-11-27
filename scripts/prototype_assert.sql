DO $$
BEGIN
 IF (SELECT count(*) FROM finance.audit_event)<>2 OR (SELECT count(*) FROM finance.outbox)<>2 OR (SELECT count(*) FROM finance.idempotency_record)<>2 THEN RAISE EXCEPTION 'atomicity/deduplication mismatch'; END IF;
 IF (SELECT count(*) FROM finance.organization_unit)<>4 THEN RAISE EXCEPTION 'unexpected unit count'; END IF;
 IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='finance_runtime' AND (rolsuper OR rolbypassrls OR rolcanlogin OR rolcreaterole)) THEN RAISE EXCEPTION 'runtime role not restricted'; END IF;
 IF has_table_privilege('finance_runtime','finance.audit_event','UPDATE') OR has_table_privilege('finance_runtime','finance.audit_event','DELETE') OR has_table_privilege('finance_runtime','finance.membership','INSERT') THEN RAISE EXCEPTION 'unexpected runtime privilege'; END IF;
END $$;
BEGIN;
SET LOCAL ROLE finance_runtime;
SELECT set_config('finance.tenant_id','00000000-0000-0000-0000-000000000001',true),set_config('finance.company_id','00000000-0000-0000-0000-00000000000a',true),set_config('finance.actor_id','00000000-0000-0000-0000-0000000003e8',true);
DO $$
BEGIN
 BEGIN
 INSERT INTO finance.organization_unit(tenant_id,company_id,id,parent_id,code,name,kind) VALUES ('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000000999','00000000-0000-0000-0000-000000000401','BAD-FK','bad','DEPARTMENT');
 RAISE EXCEPTION 'cross-company FK accepted';
 EXCEPTION WHEN foreign_key_violation THEN NULL; END;
 BEGIN
 INSERT INTO finance.organization_unit(tenant_id,company_id,id,code,name,kind) VALUES ('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000b','00000000-0000-0000-0000-000000000999','BAD-RLS','bad','DEPARTMENT');
 RAISE EXCEPTION 'cross-company insert accepted';
 EXCEPTION WHEN insufficient_privilege THEN NULL; END;
END $$;
ROLLBACK;
