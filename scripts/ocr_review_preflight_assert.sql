-- Uses real OCR evidence created by real_persist.rs, not fabricated extracted fields.
DO $$
DECLARE t uuid:='00000000-0000-0000-0000-000000000001';c uuid:='00000000-0000-0000-0000-00000000000a';
 d uuid:='00000000-0000-0000-0000-000000001f41';r uuid:='00000000-0000-0000-0000-000000001f44';
 rev uuid:='00000000-0000-0000-0000-000000001f42'; checker uuid:=gen_random_uuid(); unassigned uuid:=gen_random_uuid();
 owner_id uuid:='00000000-0000-0000-0000-0000000003e9';a uuid:=gen_random_uuid();review uuid:=gen_random_uuid();
 check1 uuid:=gen_random_uuid();check2 uuid:=gen_random_uuid();hash1 bytea;hash2 bytea;day date:='2027-01-01';
BEGIN
 INSERT INTO finance.identity_subject(tenant_id,id,issuer,subject) VALUES(t,a,'https://synthetic.invalid','synthetic-accountant'),(t,checker,'https://synthetic.invalid','synthetic-review-assigner'),(t,unassigned,'https://synthetic.invalid','synthetic-unassigned');
 INSERT INTO finance.membership(tenant_id,company_id,actor_id,role) VALUES(t,c,a,'ACCOUNTANT'),(t,c,owner_id,'ACCOUNTANT'),(t,c,unassigned,'ACCOUNTANT'),(t,c,checker,'FINANCE_APPROVER');
 PERFORM finance.prototype_assign_candidate_review(t,c,d,rev,a,checker);
 BEGIN
  PERFORM finance.prototype_review_candidate(t,c,d,0,r,'invoice.total',unassigned,review,'CONFIRMED_CANDIDATE','Unassigned review');
  RAISE EXCEPTION 'unassigned review accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 PERFORM finance.prototype_preflight(t,c,d,0,owner_id,check1,day);
 BEGIN
  PERFORM finance.prototype_review_candidate(t,c,d,0,r,'invoice.total',owner_id,review,'CONFIRMED_CANDIDATE','Self review');
  RAISE EXCEPTION 'self review accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN
  PERFORM finance.prototype_review_candidate(t,c,d,0,r,'invoice.tax',a,review,'CONFIRMED_CANDIDATE','Unknown tax');
  RAISE EXCEPTION 'unknown candidate confirmed';
 EXCEPTION WHEN check_violation THEN NULL; END;
 PERFORM finance.prototype_review_candidate(t,c,d,0,r,'invoice.total',a,review,'CONFIRMED_CANDIDATE','Checked synthetic OCR candidate');
 IF (SELECT normalized_value FROM finance.field_decision WHERE id=review) IS DISTINCT FROM '125000' THEN RAISE EXCEPTION 'confirmed value differs from OCR'; END IF;
 IF finance.prototype_review_candidate(t,c,d,0,r,'invoice.total',a,review,'CONFIRMED_CANDIDATE','Checked synthetic OCR candidate') IS DISTINCT FROM review
 THEN RAISE EXCEPTION 'review replay mismatch'; END IF;
 BEGIN
  PERFORM finance.prototype_preflight(t,c,d,0,owner_id,gen_random_uuid(),day);
  RAISE EXCEPTION 'stale preflight accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 PERFORM finance.prototype_preflight(t,c,d,1,owner_id,check2,day);
 SELECT input_hash INTO hash1 FROM finance.check_run WHERE id=check1;
 SELECT input_hash INTO hash2 FROM finance.check_run WHERE id=check2;
 IF hash1=hash2 OR (SELECT count(*) FROM finance.field_decision)<>1
 OR (SELECT count(*) FROM finance.check_run WHERE blocked)<>2
 OR NOT EXISTS(SELECT 1 FROM finance.check_result WHERE run_id=check2 AND rule_key='scan.release' AND outcome='UNKNOWN' AND severity='BLOCK')
 OR (SELECT status FROM finance.dossier WHERE id=d)<>'DRAFT'
 OR (SELECT count(*) FROM finance.audit_event WHERE action='SYNTHETIC_FIELD_REVIEWED')<>1
 OR has_table_privilege('finance_runtime','finance.field_decision','INSERT')
 THEN RAISE EXCEPTION 'review/preflight invariant failed'; END IF;
 BEGIN
  UPDATE finance.field_decision SET reason='changed' WHERE id=review;
  RAISE EXCEPTION 'decision mutated';
 EXCEPTION WHEN check_violation THEN NULL; END;
END $$;
SELECT 'PASS: actual OCR evidence review, independent accountant, immutable decision, stale preflight rejection and blocked unapproved scan';
