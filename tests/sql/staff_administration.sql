-- Disposable database fixture only. Never execute against production.
\set t '00000000-0000-0000-0000-000000000001'
\set c '00000000-0000-0000-0000-000000000010'
\set a '00000000-0000-0000-0000-000000001000'
\set b '00000000-0000-0000-0000-000000001001'
\set d '00000000-0000-0000-0000-000000000100'
\set p '00000000-0000-0000-0000-000000000200'
\set r '00000000-0000-0000-0000-000000000300'
\set j '00000000-0000-0000-0000-000000000400'
\set g '00000000-0000-0000-0000-000000000500'
\set i '00000000-0000-0000-0000-000000000600'
BEGIN;
INSERT INTO finance.tenant(id,code) VALUES(:'t','STAFF_TEST');
INSERT INTO finance.company(tenant_id,id,code,name) VALUES(:'t',:'c','TEST','Test');
INSERT INTO finance.identity_subject(tenant_id,id,issuer,subject) VALUES(:'t',:'a','https://synthetic.invalid','admin'),(:'t',:'b','https://synthetic.invalid','employee');
INSERT INTO finance.membership(tenant_id,company_id,actor_id,role) VALUES(:'t',:'c',:'a','ORG_ADMIN'),(:'t',:'c',:'b','REQUESTER');
INSERT INTO finance.organization_unit(tenant_id,company_id,id,code,name,kind) VALUES(:'t',:'c',:'d','D1','Dept','DEPARTMENT');
SELECT set_config('finance.tenant_id',:'t',true),set_config('finance.company_id',:'c',true),set_config('finance.actor_id',:'a',true);
SET LOCAL ROLE finance_runtime;
SELECT finance.staff_admin_save(:'t',:'c',:'a',gen_random_uuid(),gen_random_uuid(),'pos','positions',:'p',NULL,'{"name":"Lead"}');
SELECT finance.staff_admin_save(:'t',:'c',:'a',gen_random_uuid(),gen_random_uuid(),'pos','positions',:'p',NULL,'{"name":"Lead"}');
SELECT finance.staff_admin_save(:'t',:'c',:'a',gen_random_uuid(),gen_random_uuid(),'dept','departments',:'j',NULL,jsonb_build_object('actor_id',:'b','department_id',:'d','position_id',:'p','is_primary',true));
SELECT finance.staff_admin_save(:'t',:'c',:'a',gen_random_uuid(),gen_random_uuid(),'role','roles',:'r',NULL,'{"name":"Department reader","capabilities":["READ_DEPARTMENT"]}');
SELECT finance.staff_admin_save(:'t',:'c',:'a',gen_random_uuid(),gen_random_uuid(),'assign','assignments',:'g',NULL,jsonb_build_object('actor_id',:'b','department_id',:'d','role_id',:'r','role_version',0));
SELECT finance.staff_admin_save(:'t',:'c',:'a',gen_random_uuid(),gen_random_uuid(),'invite','invitations',:'i',NULL,'{"email":"new@example.invalid"}');
DO $$
DECLARE t uuid:=finance.scope_tenant();c uuid:=finance.scope_company();a uuid:=finance.scope_actor();nil uuid:='00000000-0000-0000-0000-000000000000';
BEGIN
 IF (SELECT count(*) FROM finance.staff_admin_list(t,c,a,'positions',nil,100))<>1 OR (SELECT count(*) FROM finance.staff_admin_list(t,c,a,'assignments',nil,100))<>1 OR (SELECT count(*) FROM finance.staff_admin_list(t,c,a,'invitations',nil,100))<>1 THEN RAISE EXCEPTION 'list mismatch'; END IF;
 BEGIN
  PERFORM finance.staff_admin_save(t,c,a,gen_random_uuid(),gen_random_uuid(),'bad','roles',gen_random_uuid(),NULL,'{"name":"Bad","capabilities":["ORG_ADMIN"]}');
  RAISE EXCEPTION 'accepted privileged role';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN
  PERFORM finance.staff_admin_save(t,c,a,gen_random_uuid(),gen_random_uuid(),'pos','positions','00000000-0000-0000-0000-000000000200',NULL,'{"name":"Other"}');
  RAISE EXCEPTION 'accepted conflicting retry';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN
  PERFORM finance.staff_admin_save(t,c,a,gen_random_uuid(),gen_random_uuid(),'stale','positions','00000000-0000-0000-0000-000000000200',2,'{"name":"Stale"}');
  RAISE EXCEPTION 'accepted stale version';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN
  PERFORM finance.staff_admin_list(t,gen_random_uuid(),a,'roles',nil,100);
  RAISE EXCEPTION 'accepted wrong company';
 EXCEPTION WHEN insufficient_privilege THEN NULL; END;
 BEGIN
  PERFORM finance.staff_admin_save(t,c,a,gen_random_uuid(),gen_random_uuid(),'wide','assignments',gen_random_uuid(),NULL,'{"actor_id":"00000000-0000-0000-0000-000000001001","role_id":"00000000-0000-0000-0000-000000000300","role_version":0}');
  RAISE EXCEPTION 'accepted company-wide department role';
 EXCEPTION WHEN check_violation THEN NULL; END;
END $$;
SELECT finance.staff_admin_save(:'t',:'c',:'a',gen_random_uuid(),gen_random_uuid(),'revise','roles',:'r',0,'{"name":"Reader revised","capabilities":["READ_OWN"]}');
SELECT finance.staff_admin_save(:'t',:'c',:'a',gen_random_uuid(),gen_random_uuid(),'revoke','assignments',:'g',0,'{}');
SELECT finance.staff_admin_save(:'t',:'c',:'a',gen_random_uuid(),gen_random_uuid(),'cancel','invitations',:'i',0,'{}');
RESET ROLE;
DO $$ BEGIN
 IF (SELECT count(*) FROM finance.staff_admin_change)<>8 THEN RAISE EXCEPTION 'audit count mismatch'; END IF;
 IF (SELECT count(*) FROM finance.staff_role_revision)<>2 THEN RAISE EXCEPTION 'role history mismatch'; END IF;
END $$;
SELECT set_config('finance.actor_id',:'b',true);
SET LOCAL ROLE finance_runtime;
DO $$ BEGIN
 BEGIN
  PERFORM finance.staff_admin_list(finance.scope_tenant(),finance.scope_company(),finance.scope_actor(),'roles','00000000-0000-0000-0000-000000000000',100);
  RAISE EXCEPTION 'non-admin read';
 EXCEPTION WHEN insufficient_privilege THEN NULL; END;
END $$;
ROLLBACK;
SELECT 'PASS staff administration isolated fixture';
