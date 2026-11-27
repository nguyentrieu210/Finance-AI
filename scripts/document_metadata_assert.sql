-- Invoked on the existing disposable integration DB only, as its fixture owner.
DO $$
DECLARE t uuid:='00000000-0000-0000-0000-000000000001'; c uuid:='00000000-0000-0000-0000-00000000000a';
 a uuid:='00000000-0000-0000-0000-0000000003e9';
 d uuid:='00000000-0000-0000-0000-000000009001'; r uuid:='00000000-0000-0000-0000-000000009002';
 i uuid:='00000000-0000-0000-0000-000000009003'; g bigint;
BEGIN
 PERFORM finance.prototype_reserve_upload(t,c,i,d,r,a,'synthetic.pdf',26214400,decode(repeat('aa',32),'hex'),'application/pdf');
 g:=finance.prototype_issue_upload_grant(t,c,i,r,a,300);
 IF g<>1 OR NOT finance.prototype_upload_grant_current(t,c,i,r,a,g) THEN RAISE EXCEPTION 'first grant invalid'; END IF;
 g:=finance.prototype_issue_upload_grant(t,c,i,r,a,300);
 IF g<>2 OR finance.prototype_upload_grant_current(t,c,i,r,a,1) OR NOT finance.prototype_upload_grant_current(t,c,i,r,a,2) THEN RAISE EXCEPTION 'generation fence mismatch'; END IF;
 IF finance.prototype_upload_grant_current(t,c,i,gen_random_uuid(),a,2) OR finance.prototype_upload_grant_current(t,'00000000-0000-0000-0000-00000000000b',i,r,a,2) THEN RAISE EXCEPTION 'scope leak'; END IF;
 BEGIN
  PERFORM finance.prototype_issue_upload_grant(t,c,i,r,a,301);
  RAISE EXCEPTION 'excessive TTL accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 -- Force expiration in fixture owner context only, preserving the TTL constraint.
 UPDATE finance.upload_grant SET issued_at=clock_timestamp()-interval '2 seconds',expires_at=clock_timestamp()-interval '1 second' WHERE tenant_id=t AND company_id=c AND intent_id=i AND generation=2;
 IF finance.prototype_upload_grant_current(t,c,i,r,a,2) THEN RAISE EXCEPTION 'expired grant accepted'; END IF;
 FOR j IN 1..3 LOOP
  PERFORM finance.prototype_reserve_upload(t,c,gen_random_uuid(),d,r,a,'synthetic.pdf',26214400,decode(repeat('aa',32),'hex'),'application/pdf');
 END LOOP;
 BEGIN
  PERFORM finance.prototype_reserve_upload(t,c,gen_random_uuid(),d,gen_random_uuid(),a,'overflow.pdf',1,decode(repeat('aa',32),'hex'),'application/pdf');
  RAISE EXCEPTION 'quota overflow accepted across revisions';
 EXCEPTION WHEN check_violation THEN NULL; END;
 PERFORM finance.prototype_cancel_upload(t,c,i,r,a);
 IF finance.prototype_upload_grant_current(t,c,i,r,a,2) THEN RAISE EXCEPTION 'cancelled grant accepted'; END IF;
 BEGIN
  PERFORM finance.prototype_issue_upload_grant(t,c,i,r,a,300);
  RAISE EXCEPTION 'cancelled intent reopened';
 EXCEPTION WHEN check_violation THEN NULL; END;
 PERFORM finance.prototype_reserve_upload(t,c,gen_random_uuid(),d,r,a,'replacement.pdf',26214400,decode(repeat('aa',32),'hex'),'application/pdf');
 IF (SELECT sum(declared_bytes) FROM finance.upload_intent WHERE dossier_id=d AND state='PENDING_UPLOAD')<>104857600 THEN RAISE EXCEPTION 'quota release mismatch'; END IF;
 IF has_table_privilege('finance_runtime','finance.upload_intent','UPDATE') OR has_table_privilege('finance_job_worker','finance.upload_grant','SELECT') OR has_function_privilege('finance_runtime','finance.prototype_reserve_upload(uuid,uuid,uuid,uuid,uuid,uuid,text,bigint,bytea,text)','EXECUTE') THEN RAISE EXCEPTION 'prototype metadata exposed'; END IF;
END $$;
