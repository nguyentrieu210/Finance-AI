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
 item:='{"source_key":"ADV1","cap":"100","consumed":"50","details":{"beneficiary_key":"EMP1","purpose":"SYN","issued_on":"2026-11-01","due_date":"2026-11-20","issued":"100","settled":"30","refunded":"20","extension":{"approved_on":"2026-11-19","new_due":"2026-12-01","approval_ref":"SYN-EXT"}}}'::jsonb;
 IF NOT finance.validate_reference_details('ADVANCE',item,day) OR finance.validate_reference_details('ADVANCE',jsonb_set(item,'{consumed}','"30"'),day) THEN RAISE EXCEPTION 'refund accounting'; END IF;
END $$;
