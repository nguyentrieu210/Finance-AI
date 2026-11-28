DO $$
DECLARE t uuid:='00000000-0000-0000-0000-000000000001'; c uuid:='00000000-0000-0000-0000-00000000000a';
 maker uuid:='00000000-0000-0000-0000-000000001501'; checker uuid:='00000000-0000-0000-0000-000000001502';
 p1 uuid:='00000000-0000-0000-0000-000000001503'; p2 uuid:='00000000-0000-0000-0000-000000001504';
 cl uuid:='00000000-0000-0000-0000-000000001505'; it uuid:='00000000-0000-0000-0000-000000001506';
 h bytea:=decode(repeat('11',32),'hex');
BEGIN
 INSERT INTO finance.identity_subject(tenant_id,id,issuer,subject) VALUES(t,maker,'https://synthetic.invalid','policy-maker'),(t,checker,'https://synthetic.invalid','policy-checker');
 INSERT INTO finance.membership(tenant_id,company_id,actor_id,role) VALUES(t,c,maker,'POLICY_ADMIN'),(t,c,checker,'FINANCE_APPROVER');
 INSERT INTO finance.policy_version(tenant_id,company_id,id,dossier_kind,author_id,effective_from,effective_until,source_digest)
 VALUES(t,c,p1,'PAYMENT',maker,'2027-01-01','2028-01-01',h),(t,c,p2,'PAYMENT',maker,'2027-06-01','2028-06-01',h);
 BEGIN
  PERFORM finance.prototype_policy_transition(t,c,p1,maker,'IN_REVIEW');
  RAISE EXCEPTION 'unmapped policy accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 INSERT INTO finance.policy_clause VALUES(t,c,p1,cl,'P1','Synthetic mandatory document',h,1),(t,c,p2,cl,'P2','Synthetic mandatory document',h,1);
 INSERT INTO finance.checklist_item VALUES(t,c,p1,it,cl,'INVOICE',true),(t,c,p2,it,cl,'INVOICE',true);
 PERFORM finance.prototype_policy_transition(t,c,p1,maker,'IN_REVIEW');
 PERFORM finance.prototype_policy_transition(t,c,p2,maker,'IN_REVIEW');
 BEGIN
  PERFORM finance.prototype_policy_transition(t,c,p1,maker,'PUBLISHED');
  RAISE EXCEPTION 'self publish accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 PERFORM finance.prototype_policy_transition(t,c,p1,checker,'PUBLISHED');
 BEGIN
  PERFORM finance.prototype_policy_transition(t,c,p2,checker,'PUBLISHED');
  RAISE EXCEPTION 'overlapping policy accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN
  UPDATE finance.policy_clause SET excerpt='changed' WHERE policy_id=p1;
  RAISE EXCEPTION 'published clause changed';
 EXCEPTION WHEN check_violation THEN NULL; END;
 IF finance.prototype_effective_policy(t,c,'PAYMENT','2027-06-01') IS DISTINCT FROM p1
 OR finance.prototype_effective_policy(t,c,'ADVANCE','2027-06-01') IS NOT NULL THEN
  RAISE EXCEPTION 'effective policy resolution wrong'; END IF;
 PERFORM finance.prototype_policy_transition(t,c,p1,checker,'REVOKED');
 IF finance.prototype_effective_policy(t,c,'PAYMENT','2027-06-01') IS NOT NULL THEN
  RAISE EXCEPTION 'revoked policy effective'; END IF;
 IF has_table_privilege('finance_runtime','finance.policy_version','INSERT')
 OR has_function_privilege('finance_runtime','finance.prototype_policy_transition(uuid,uuid,uuid,uuid,text)','EXECUTE') THEN
  RAISE EXCEPTION 'policy fixture exposed'; END IF;
END $$;
SELECT 'PASS: synthetic clause mapping, independent checker, overlap rejection, immutable content and revocation';
