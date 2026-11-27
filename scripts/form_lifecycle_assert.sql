DO $$
DECLARE t uuid:='00000000-0000-0000-0000-000000000001';
 c uuid:='00000000-0000-0000-0000-00000000000a';
 p uuid:='00000000-0000-0000-0000-000000001900';
 maker uuid:='00000000-0000-0000-0000-0000000003f0';
 checker uuid:='00000000-0000-0000-0000-0000000003f1';
 requester uuid:='00000000-0000-0000-0000-0000000003e9';
 fid uuid; latest bigint;
BEGIN
 SELECT form_id,max(version) INTO fid,latest FROM finance.form_version
 WHERE tenant_id=t AND company_id=c AND policy_id=p GROUP BY form_id;
 IF fid IS NULL OR latest<>2 THEN RAISE EXCEPTION 'form API did not create two immutable versions'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.form_version WHERE form_id=fid AND version=1 AND schema_json#>>'{fields,0,source_path}'='request.advance')
 OR NOT EXISTS(SELECT 1 FROM finance.form_version WHERE form_id=fid AND version=2 AND schema_json#>>'{fields,0,layout,width}'='12')
 THEN RAISE EXCEPTION 'normalized source/layout missing'; END IF;
 PERFORM finance.prototype_policy_transition(t,c,p,maker,'IN_REVIEW');
 PERFORM finance.prototype_policy_transition(t,c,p,checker,'PUBLISHED');
 IF (SELECT state FROM finance.form_version WHERE form_id=fid AND version=1)<>'RETIRED'
 OR (SELECT state FROM finance.form_version WHERE form_id=fid AND version=2)<>'PUBLISHED'
 THEN RAISE EXCEPTION 'policy publication did not pin latest form version'; END IF;
 BEGIN
  UPDATE finance.form_version SET name='tampered' WHERE form_id=fid AND version=2;
  RAISE EXCEPTION 'published form content mutated';
 EXCEPTION WHEN check_violation THEN NULL; END;
 IF (SELECT count(*) FROM finance.audit_event WHERE resource_id=fid AND action IN ('FORM_DRAFT_CREATED','FORM_DRAFT_REVISED'))<>2
 THEN RAISE EXCEPTION 'form audit coverage missing'; END IF;
 PERFORM set_config('finance.tenant_id',t::text,true);
 PERFORM set_config('finance.company_id',c::text,true);
 PERFORM set_config('finance.actor_id',requester::text,true);
 EXECUTE 'SET LOCAL ROLE finance_runtime';
 IF (SELECT count(*) FROM finance.form_version WHERE form_id=fid)<>1
 OR NOT EXISTS(SELECT 1 FROM finance.form_version WHERE form_id=fid AND state='PUBLISHED')
 THEN RAISE EXCEPTION 'requester form RLS did not expose exactly published version'; END IF;
 IF has_table_privilege('finance_runtime','finance.form_version','UPDATE')
 OR has_table_privilege('finance_runtime','finance.form_version','DELETE')
 THEN RAISE EXCEPTION 'runtime form mutation grant'; END IF;
END $$;
SELECT 'PASS: immutable form versions, policy-coupled maker-checker publish and requester published-only RLS';
