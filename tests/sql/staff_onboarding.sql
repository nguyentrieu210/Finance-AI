-- Isolated fixture only, rolled back.
\set t '00000000-0000-0000-0000-000000000001'
\set c '00000000-0000-0000-0000-000000000010'
\set a '00000000-0000-0000-0000-000000001000'
\set i '00000000-0000-0000-0000-000000000600'
\set r '00000000-0000-0000-0000-000000000300'
BEGIN;
INSERT INTO finance.tenant(id,code) VALUES(:'t','ONBOARD_TEST');
INSERT INTO finance.company(tenant_id,id,code,name) VALUES(:'t',:'c','TEST','Test');
INSERT INTO finance.identity_subject(tenant_id,id,issuer,subject) VALUES(:'t',:'a','https://synthetic.invalid','admin');
INSERT INTO finance.membership(tenant_id,company_id,actor_id,role) VALUES(:'t',:'c',:'a','ORG_ADMIN');
SELECT set_config('finance.tenant_id',:'t',true),set_config('finance.company_id',:'c',true),set_config('finance.actor_id',:'a',true);
SET LOCAL ROLE finance_runtime;
SELECT finance.staff_admin_save(:'t',:'c',:'a',gen_random_uuid(),gen_random_uuid(),'invite','invitations',:'i',NULL,'{"email":"new@example.invalid"}');
SELECT finance.staff_invitation_issue(:'t',:'c',:'a',:'i',0,true);
RESET ROLE;
SET LOCAL ROLE finance_identity_lookup;
SELECT set_config('finance.identity_issuer','https://synthetic.invalid',true),set_config('finance.identity_subject','new-employee',true);
DO $$ BEGIN
 IF finance.staff_onboarding_claim(finance.scope_tenant(),finance.scope_company(),'https://synthetic.invalid','new-employee','other@example.invalid','Other',1,extract(epoch FROM now()+interval '5 minutes')::bigint) IS NOT NULL THEN RAISE EXCEPTION 'accepted wrong email'; END IF;
END $$;
SELECT finance.staff_onboarding_claim(:'t',:'c','https://synthetic.invalid','new-employee','new@example.invalid','New Employee',1,extract(epoch FROM now()+interval '5 minutes')::bigint) AS rid \gset
SELECT finance.staff_onboarding_claim(:'t',:'c','https://synthetic.invalid','new-employee','new@example.invalid','New Employee',1,extract(epoch FROM now()+interval '5 minutes')::bigint);
RESET ROLE;
DO $$ BEGIN
 IF (SELECT count(*) FROM finance.identity_subject)<>1 OR (SELECT count(*) FROM finance.membership)<>1 THEN RAISE EXCEPTION 'claim granted access'; END IF;
END $$;
SET LOCAL ROLE finance_runtime;
SELECT finance.staff_onboarding_list(:'t',:'c',:'a','00000000-0000-0000-0000-000000000000',100);
SELECT finance.staff_onboarding_decide(:'t',:'c',:'a',:'rid',0,true);
RESET ROLE;
SELECT id AS b FROM finance.identity_subject WHERE subject='new-employee' \gset
DO $$ BEGIN
 IF (SELECT count(*) FROM finance.membership WHERE role='REQUESTER')<>1 OR (SELECT count(*) FROM finance.membership WHERE role='ORG_ADMIN')<>1 THEN RAISE EXCEPTION 'approval role mismatch'; END IF;
END $$;
SET LOCAL ROLE finance_runtime;
SELECT finance.staff_admin_save(:'t',:'c',:'a',gen_random_uuid(),gen_random_uuid(),'role','roles',:'r',NULL,'{"name":"Own","capabilities":["READ_OWN"]}');
SELECT finance.staff_admin_save(:'t',:'c',:'a',gen_random_uuid(),gen_random_uuid(),'assign','assignments',gen_random_uuid(),NULL,jsonb_build_object('actor_id',:'b','role_id',:'r','role_version',0));
SELECT set_config('finance.actor_id',:'b',true);
DO $$ BEGIN
 IF (SELECT count(*) FROM finance.staff_effective_grants(finance.scope_tenant(),finance.scope_company(),finance.scope_actor()))<>1 THEN RAISE EXCEPTION 'missing grant'; END IF;
 IF EXISTS(SELECT 1 FROM finance.staff_effective_grants(finance.scope_tenant(),gen_random_uuid(),finance.scope_actor())) THEN RAISE EXCEPTION 'cross scope grant'; END IF;
END $$;
SELECT set_config('finance.actor_id',:'a',true);
SELECT finance.staff_admin_save(:'t',:'c',:'a',gen_random_uuid(),gen_random_uuid(),'role2','roles',:'r',0,'{"name":"Own revised","capabilities":["READ_OWN","WRITE_OWN"]}');
SELECT set_config('finance.actor_id',:'b',true);
DO $$ BEGIN
 IF EXISTS(SELECT 1 FROM finance.staff_effective_grants(finance.scope_tenant(),finance.scope_company(),finance.scope_actor())) THEN RAISE EXCEPTION 'stale role revision grants'; END IF;
END $$;
ROLLBACK;
SELECT 'PASS onboarding and effective grants';
