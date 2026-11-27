DO $$
DECLARE t uuid:='00000000-0000-0000-0000-000000000001';c uuid:='00000000-0000-0000-0000-00000000000a';
 maker uuid:='00000000-0000-0000-0000-000000001501';checker uuid:='00000000-0000-0000-0000-000000001502';
 s uuid:=gen_random_uuid();v uuid:=gen_random_uuid();k uuid:=gen_random_uuid();i uuid:=gen_random_uuid();a uuid:=gen_random_uuid();b uuid:=gen_random_uuid();
 r uuid:=gen_random_uuid();d date:='2027-01-01'; h bytea:=decode(repeat('aa',32),'hex');
BEGIN
 INSERT INTO finance.reference_snapshot(tenant_id,company_id,id,source_system,external_version,source_digest,as_of,fresh_until,maker_id)
 VALUES(t,c,s,'SYNTHETIC','V1',h,clock_timestamp()-interval '1 minute',clock_timestamp()+interval '1 hour',maker);
 INSERT INTO finance.vendor_ref VALUES(t,c,s,v,'VENDOR1','Synthetic vendor');
 INSERT INTO finance.contract_ref VALUES(t,c,s,k,'CONTRACT1',v,'VND',100,20,d,d+365);
 INSERT INTO finance.invoice_ref VALUES(t,c,s,i,v,'SERIES','000001','VND',90,10,d);
 INSERT INTO finance.advance_ref VALUES(t,c,s,a,'ADVANCE1',maker,'VND',100,20,10,d+30);
 INSERT INTO finance.budget_ref VALUES(t,c,s,b,'CC1','ACC1','VND',200,100,d,d+365);
 INSERT INTO finance.source_coverage VALUES(t,c,s,'CONTRACT',true,'W1'),(t,c,s,'INVOICE',true,'W1'),(t,c,s,'ADVANCE',false,'W1'),(t,c,s,'BUDGET',true,'W1');
 BEGIN
  PERFORM finance.prototype_approve_reference(t,c,s,maker,d);
  RAISE EXCEPTION 'self approval accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN
  PERFORM finance.prototype_approve_reference(t,c,s,checker,d);
  RAISE EXCEPTION 'incomplete coverage accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 IF EXISTS(SELECT 1 FROM finance.financial_resource WHERE snapshot_id=s) THEN RAISE EXCEPTION 'partial import'; END IF;
 UPDATE finance.source_coverage SET complete=true WHERE snapshot_id=s AND resource_kind='ADVANCE';
 PERFORM finance.prototype_approve_reference(t,c,s,checker,d);
 IF (SELECT count(*) FROM finance.financial_resource WHERE snapshot_id=s)<>4 OR (SELECT consumed FROM finance.financial_resource WHERE id=a)<>30 THEN RAISE EXCEPTION 'import balance mismatch'; END IF;
 BEGIN
  UPDATE finance.invoice_ref SET invoice_number='changed' WHERE snapshot_id=s;
  RAISE EXCEPTION 'approved reference mutated';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN
  PERFORM finance.prototype_reserve_finance(t,c,r,gen_random_uuid(),'VND',71,ARRAY[k,i,a,b]);
  RAISE EXCEPTION 'advance cap ignored';
 EXCEPTION WHEN check_violation THEN NULL; END;
 PERFORM finance.prototype_reserve_finance(t,c,r,gen_random_uuid(),'VND',70,ARRAY[k,i,a,b]);
 PERFORM finance.prototype_revoke_reference(t,c,s,checker);
 IF (SELECT remaining FROM finance.reservation WHERE id=r)<>70 THEN RAISE EXCEPTION 'revocation released hold'; END IF;
 BEGIN
  PERFORM finance.prototype_reserve_finance(t,c,gen_random_uuid(),gen_random_uuid(),'VND',1,ARRAY[k]);
  RAISE EXCEPTION 'revoked coverage reserved';
 EXCEPTION WHEN check_violation THEN NULL; END;
 IF has_table_privilege('finance_runtime','finance.reference_snapshot','SELECT') OR has_function_privilege('finance_runtime','finance.prototype_approve_reference(uuid,uuid,uuid,uuid,date)','EXECUTE') THEN RAISE EXCEPTION 'fixture import exposed'; END IF;
END $$;
SELECT 'PASS: scoped reference import, incomplete coverage denial, independent checker, multi-cap and revocation preserves holds';
