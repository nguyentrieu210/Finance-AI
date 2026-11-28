-- Run against migrated disposable test DB as schema owner; no real financial data.
DO $$
DECLARE day date:='2026-11-28'; item jsonb;
BEGIN
 item:='{"source_key":"INSTALL-1","cap":"90","consumed":"20","details":{"contract_key":"SYN-C","installment_key":"INSTALL-1","vendor_key":"SYN-V","effective_from":"2026-01-01","effective_until":"2027-01-01","base_cap":"100","addenda":[{"key":"FUTURE","approved_on":"2026-11-01","effective_on":"2026-12-01","delta":"20"}],"installment_gross":"100","retention":"10","advance_deduction":"0","paid":"20"}}'::jsonb;
 IF NOT finance.validate_reference_details('CONTRACT',item,day) THEN RAISE EXCEPTION 'contract installment rejected'; END IF;
 IF finance.validate_reference_details('CONTRACT',jsonb_set(item,'{cap}','"110"'),day) THEN RAISE EXCEPTION 'future addendum counted'; END IF;
 IF finance.reference_details_eligible('CONTRACT',(item->'details')||'{"frozen_on":"2026-11-28"}'::jsonb,'2026-12-01') THEN RAISE EXCEPTION 'changed addendum requires refreshed snapshot'; END IF;
 item:='{"source_key":"B1","cap":"80","consumed":"30","details":{"cost_center":"CC1","account":"A1","budget_version":"V1","period_start":"2026-11-01","period_end":"2026-12-01","approved_limit":"100","external_unmatched_commitments":"20","actual":"30","disjoint_verified":true}}'::jsonb;
 IF NOT finance.validate_reference_details('BUDGET',item,day) OR finance.reference_details_eligible('BUDGET',item->'details','2026-12-01') THEN RAISE EXCEPTION 'budget boundary'; END IF;
 IF finance.validate_reference_details('BUDGET',jsonb_set(item,'{details,disjoint_verified}','false'),day) THEN RAISE EXCEPTION 'overlapping buckets accepted'; END IF;
 IF finance.validate_reference_details('BUDGET',jsonb_set(item,'{details,unknown_field}','true'),day) THEN RAISE EXCEPTION 'undeclared reference field accepted'; END IF;
 item:='{"source_key":"ADV1","cap":"100","consumed":"50","details":{"beneficiary_key":"EMP1","purpose":"SYN","issued_on":"2026-11-01","due_date":"2026-11-20","issued":"100","settled":"30","refunded":"20","extension":{"approved_on":"2026-11-19","new_due":"2026-12-01","approval_ref":"SYN-EXT"}}}'::jsonb;
 IF NOT finance.validate_reference_details('ADVANCE',item,day) OR finance.validate_reference_details('ADVANCE',jsonb_set(item,'{consumed}','"30"'),day) THEN RAISE EXCEPTION 'refund accounting'; END IF;
END $$;

DO $$
DECLARE lines jsonb:='[{"key":"RECEIPT-1:LINE-1","item":"SYN-ITEM","unit":"kg","quantity":"10","unit_price":"2","value":"20"}]'::jsonb;
BEGIN
 IF NOT finance.valid_acceptance_lines(lines) THEN RAISE EXCEPTION 'valid receipt rejected'; END IF;
 IF finance.valid_acceptance_lines(lines||lines) THEN RAISE EXCEPTION 'duplicate receipt identity accepted'; END IF;
 IF finance.valid_acceptance_lines(jsonb_set(lines,'{0,value}','"21"')) THEN RAISE EXCEPTION 'inexact receipt total accepted'; END IF;
 IF finance.valid_acceptance_lines(jsonb_set(lines,'{0,quantity}','"0"')) THEN RAISE EXCEPTION 'zero receipt quantity accepted'; END IF;
 IF has_table_privilege('finance_runtime','finance.acceptance_claim','INSERT') OR has_table_privilege('finance_runtime','finance.settlement_extra_claim','INSERT')
 OR has_function_privilege('finance_runtime','finance.seal_acceptance_claims(uuid,uuid,uuid,uuid,uuid,uuid,uuid)','EXECUTE')
 OR has_function_privilege('finance_runtime','finance.seal_extra_settlement(uuid,uuid,uuid,uuid,uuid,uuid,bigint,uuid)','EXECUTE')
 THEN RAISE EXCEPTION 'internal claim bypass'; END IF;
END $$;
BEGIN;
INSERT INTO finance.tenant(id,code) VALUES('10000000-0000-0000-0000-000000000001','WAVE1_SYN');
INSERT INTO finance.company(tenant_id,id,code,name) VALUES('10000000-0000-0000-0000-000000000001','10000000-0000-0000-0000-000000000002','W1','Synthetic');
DO $$
DECLARE t uuid:='10000000-0000-0000-0000-000000000001'; c uuid:='10000000-0000-0000-0000-000000000002'; a uuid:=gen_random_uuid(); b uuid:=gen_random_uuid(); s uuid:=gen_random_uuid(); detail jsonb;
BEGIN
 INSERT INTO finance.identity_subject(tenant_id,id,issuer,subject) VALUES(t,a,'https://synthetic.invalid','wave-maker'),(t,b,'https://synthetic.invalid','wave-checker');
 INSERT INTO finance.reference_snapshot(tenant_id,company_id,id,source_system,external_version,source_digest,as_of,fresh_until,maker_id)
 VALUES(t,c,s,'SYNTHETIC','W1',sha256('synthetic'::bytea),clock_timestamp()-interval '1 minute',clock_timestamp()+interval '1 day',a);
 INSERT INTO finance.source_coverage(tenant_id,company_id,snapshot_id,resource_kind,complete,watermark) VALUES(t,c,s,'CONTRACT',true,'W1');
 detail:='{"contract_key":"C-1","vendor_key":"V-1","base_cap":"100","addenda":[],"effective_from":"2026-01-01","effective_until":"2027-01-01","frozen_on":"2026-11-28","installment_gross":"60","acceptance_lines":[]}'::jsonb;
 INSERT INTO finance.financial_resource(tenant_id,company_id,id,currency,cap,consumed,coverage_complete,fresh_until,snapshot_id,resource_kind,source_key,reference_details)
 VALUES(t,c,gen_random_uuid(),'VND',60,0,true,clock_timestamp()+interval '1 day',s,'CONTRACT','I1',detail);
 PERFORM finance.assert_contract_schedule(t,c,'C-1');
 BEGIN
  INSERT INTO finance.financial_resource(tenant_id,company_id,id,currency,cap,consumed,coverage_complete,fresh_until,snapshot_id,resource_kind,source_key,reference_details)
  VALUES(t,c,gen_random_uuid(),'VND',60,0,true,clock_timestamp()+interval '1 day',s,'CONTRACT','I2',detail);
  PERFORM finance.assert_contract_schedule(t,c,'C-1');
  RAISE EXCEPTION 'aggregate contract cap exceeded without rejection';
 EXCEPTION WHEN check_violation THEN NULL; END;
 IF (SELECT count(*) FROM finance.financial_resource WHERE tenant_id=t AND company_id=c)<>1 THEN RAISE EXCEPTION 'failed contract check was not atomic'; END IF;
END $$;
ROLLBACK;
