-- Disposable DB, executed after the concurrent reserve fixture.
DO $$
DECLARE t uuid:='00000000-0000-0000-0000-000000000001'; c uuid:='00000000-0000-0000-0000-00000000000a';
 r uuid:='00000000-0000-0000-0000-000000001200'; i uuid;
 a uuid:='00000000-0000-0000-0000-000000001101'; b uuid:='00000000-0000-0000-0000-000000001102';
 e1 uuid:='00000000-0000-0000-0000-000000001401'; e2 uuid:='00000000-0000-0000-0000-000000001402';
 e3 uuid:='00000000-0000-0000-0000-000000001403'; e4 uuid:='00000000-0000-0000-0000-000000001404';
 e5 uuid:='00000000-0000-0000-0000-000000001405'; e6 uuid:='00000000-0000-0000-0000-000000001406';
 h bytea:=decode(repeat('aa',32),'hex');
BEGIN
 SELECT id INTO STRICT i FROM finance.reservation WHERE tenant_id=t AND company_id=c;
 PERFORM finance.prototype_allocation(t,c,i,r,e1,'COMMIT',0,h,NULL,true,false);
 BEGIN
  PERFORM finance.prototype_allocation(t,c,i,r,e2,'RELEASE',60,h,NULL,true,false);
  RAISE EXCEPTION 'unknown external release accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 PERFORM finance.prototype_allocation(t,c,i,r,e2,'CONSUME',20,h,NULL,true,false);
 PERFORM finance.prototype_allocation(t,c,i,r,e2,'CONSUME',20,h,NULL,true,false);
 BEGIN
  PERFORM finance.prototype_allocation(t,c,i,r,e2,'CONSUME',21,h,NULL,true,false);
  RAISE EXCEPTION 'mismatched external replay accepted';
 EXCEPTION WHEN unique_violation THEN NULL; END;
 IF (SELECT remaining FROM finance.reservation WHERE id=i)<>40
 OR (SELECT state FROM finance.reservation WHERE id=i)<>'PARTIAL'
 OR (SELECT sum(consumed) FROM finance.financial_resource WHERE id IN (a,b))<>40
 THEN RAISE EXCEPTION 'partial effect or replay wrong'; END IF;
 -- 20 consumed + 40 held leaves 40; 41 must be rejected and 40 accepted.
 BEGIN
  PERFORM finance.prototype_reserve_finance(t,c,e3,r,'VND',41,ARRAY[a,b]);
  RAISE EXCEPTION 'double allocation allowed';
 EXCEPTION WHEN check_violation THEN NULL; END;
 PERFORM finance.prototype_reserve_finance(t,c,e3,r,'VND',40,ARRAY[a,b]);
 PERFORM finance.prototype_allocation(t,c,i,r,e4,'REVERSE',5,h,e2,true,true);
 IF (SELECT remaining FROM finance.reservation WHERE id=i)<>40
 OR (SELECT sum(consumed) FROM finance.financial_resource WHERE id IN (a,b))<>30
 THEN RAISE EXCEPTION 'reversal reopened hold or wrong balance'; END IF;
 BEGIN
  PERFORM finance.prototype_allocation(t,c,i,r,e5,'REVERSE',16,h,e2,true,true);
  RAISE EXCEPTION 'over reversal allowed';
 EXCEPTION WHEN check_violation THEN NULL; END;
 PERFORM finance.prototype_allocation(t,c,i,r,e5,'RELEASE',40,h,NULL,true,true);
 PERFORM finance.prototype_allocation(t,c,e3,r,e6,'CONSUME',40,h,NULL,true,true);
 IF (SELECT remaining FROM finance.reservation WHERE id=i)<>0
 OR (SELECT state FROM finance.reservation WHERE id=i)<>'RELEASED'
 OR (SELECT state FROM finance.reservation WHERE id=e3)<>'CONSUMED'
 OR (SELECT sum(consumed) FROM finance.financial_resource WHERE id IN (a,b))<>110
 OR (SELECT count(*) FROM finance.allocation_effect)<>5 THEN
  RAISE EXCEPTION 'final allocation state wrong';
 END IF;
 BEGIN
  DELETE FROM finance.allocation_effect WHERE id=e2;
  RAISE EXCEPTION 'effect deletion allowed';
 EXCEPTION WHEN check_violation THEN NULL; END;
 IF has_function_privilege('finance_runtime','finance.prototype_allocation(uuid,uuid,uuid,uuid,uuid,text,numeric,bytea,uuid,boolean,boolean)','EXECUTE')
 OR has_table_privilege('finance_runtime','finance.allocation_effect','INSERT') THEN
  RAISE EXCEPTION 'fixture kernel exposed';
 END IF;
END $$;
SELECT 'PASS: partial consume, exact replay, residual cap, reversal, reconciled release and immutable effects';
