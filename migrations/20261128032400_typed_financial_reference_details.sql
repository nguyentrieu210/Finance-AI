-- E12-E14: typed, independently reviewed financial reference dimensions.
-- Legacy generic cap rows cannot be used as detailed business evidence.
ALTER TABLE finance.financial_resource ADD COLUMN reference_details jsonb
 CHECK(reference_details IS NULL OR (jsonb_typeof(reference_details)='object' AND octet_length(reference_details::text)<=65536));
CREATE FUNCTION finance.reference_nonnegative(j jsonb,k text)
RETURNS numeric LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
DECLARE n numeric;
BEGIN
 IF jsonb_typeof(j->k) IS DISTINCT FROM 'string' OR j->>k !~ '^[0-9]{1,20}([.][0-9]{1,8})?$'
 THEN RAISE EXCEPTION 'invalid reference decimal %',k USING ERRCODE='23514'; END IF;
 n:=(j->>k)::numeric; RETURN n;
END $$;
CREATE FUNCTION finance.reference_date(j jsonb,k text)
RETURNS date LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
BEGIN
 IF jsonb_typeof(j->k) IS DISTINCT FROM 'string' OR j->>k !~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}$'
 THEN RAISE EXCEPTION 'invalid reference date %',k USING ERRCODE='23514'; END IF;
 RETURN (j->>k)::date;
END $$;
CREATE FUNCTION finance.reference_text(j jsonb,k text)
RETURNS text LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
BEGIN
 IF jsonb_typeof(j->k) IS DISTINCT FROM 'string' OR octet_length(j->>k) NOT BETWEEN 1 AND 256 OR btrim(j->>k)='' OR j->>k ~ '[[:cntrl:]]'
 THEN RAISE EXCEPTION 'invalid reference identity %',k USING ERRCODE='23514'; END IF;
 RETURN j->>k;
END $$;
CREATE FUNCTION finance.validate_reference_details(kind text,item jsonb,day date)
RETURNS boolean LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
DECLARE detail jsonb:=item->'details'; cap numeric; used numeric; gross numeric; deduction numeric; amendment jsonb; delta numeric; ext jsonb; seen text[]:=ARRAY[]::text[]; amendment_key text; due date;
BEGIN
 IF jsonb_typeof(detail) IS DISTINCT FROM 'object' OR octet_length(detail::text)>65536 THEN RETURN false; END IF;
 cap:=finance.reference_nonnegative(item,'cap'); used:=finance.reference_nonnegative(item,'consumed');
 CASE kind
 WHEN 'CONTRACT' THEN
  IF detail-ARRAY['contract_key','installment_key','vendor_key','effective_from','effective_until','base_cap','addenda','installment_gross','retention','advance_deduction','paid','acceptance_lines']<>'{}'::jsonb THEN RETURN false; END IF;
  PERFORM finance.reference_text(detail,'contract_key');PERFORM finance.reference_text(detail,'installment_key');PERFORM finance.reference_text(detail,'vendor_key');
  IF finance.reference_text(detail,'installment_key')<>item->>'source_key' OR finance.reference_date(detail,'effective_from')>=finance.reference_date(detail,'effective_until') THEN RETURN false; END IF;
  gross:=finance.reference_nonnegative(detail,'base_cap');
  IF jsonb_typeof(detail->'addenda') IS DISTINCT FROM 'array' OR jsonb_array_length(detail->'addenda')>100 THEN RETURN false; END IF;
  FOR amendment IN SELECT value FROM jsonb_array_elements(detail->'addenda') LOOP
   amendment_key:=finance.reference_text(amendment,'key');
   IF amendment_key=ANY(seen) OR finance.reference_date(amendment,'approved_on')>day
    OR amendment->>'delta' IS NULL OR amendment->>'delta' !~ '^-?[0-9]{1,20}([.][0-9]{1,8})?$' THEN RETURN false; END IF;
   seen:=array_append(seen,amendment_key);
   IF finance.reference_date(amendment,'effective_on')<=day THEN gross:=gross+(amendment->>'delta')::numeric; END IF;
  END LOOP;
  IF detail ? 'acceptance_lines' AND NOT COALESCE(finance.valid_acceptance_lines(detail->'acceptance_lines'),false) THEN RETURN false; END IF;
  deduction:=finance.reference_nonnegative(detail,'retention')+finance.reference_nonnegative(detail,'advance_deduction');
  IF gross<0 OR cap<>least(gross,finance.reference_nonnegative(detail,'installment_gross'))-deduction
   OR used<>finance.reference_nonnegative(detail,'paid') THEN RETURN false; END IF;
 WHEN 'ADVANCE' THEN
  IF detail-ARRAY['beneficiary_key','purpose','issued_on','due_date','issued','settled','refunded','extension']<>'{}'::jsonb THEN RETURN false; END IF;
  PERFORM finance.reference_text(detail,'beneficiary_key');PERFORM finance.reference_text(detail,'purpose');
  IF finance.reference_date(detail,'issued_on')>day OR finance.reference_date(detail,'due_date')<finance.reference_date(detail,'issued_on') THEN RETURN false; END IF;
  due:=finance.reference_date(detail,'due_date');
  ext:=detail->'extension';
  IF ext IS NOT NULL AND ext<>'null'::jsonb THEN
   IF jsonb_typeof(ext) IS DISTINCT FROM 'object' OR finance.reference_date(ext,'approved_on')>day OR finance.reference_date(ext,'new_due')<=due THEN RETURN false; END IF;
   PERFORM finance.reference_text(ext,'approval_ref');
  END IF;
  IF cap<>finance.reference_nonnegative(detail,'issued') OR cap<=0
   OR used<>finance.reference_nonnegative(detail,'settled')+finance.reference_nonnegative(detail,'refunded') THEN RETURN false; END IF;
 WHEN 'BUDGET' THEN
  IF detail-ARRAY['cost_center','account','budget_version','period_start','period_end','approved_limit','external_unmatched_commitments','actual','disjoint_verified']<>'{}'::jsonb THEN RETURN false; END IF;
  PERFORM finance.reference_text(detail,'cost_center');PERFORM finance.reference_text(detail,'account');PERFORM finance.reference_text(detail,'budget_version');
  IF finance.reference_date(detail,'period_start')>=finance.reference_date(detail,'period_end') OR detail->'disjoint_verified' IS DISTINCT FROM 'true'::jsonb
   OR cap<>finance.reference_nonnegative(detail,'approved_limit')-finance.reference_nonnegative(detail,'external_unmatched_commitments')
   OR used<>finance.reference_nonnegative(detail,'actual') THEN RETURN false; END IF;
 WHEN 'INVOICE' THEN
  IF detail-ARRAY['issuer','series','number','invoice_date','eligible_total','paid']<>'{}'::jsonb THEN RETURN false; END IF;
  IF finance.reference_text(detail,'issuer')||'|'||finance.reference_text(detail,'series')||'|'||finance.reference_text(detail,'number')<>item->>'source_key'
   OR position('|' in detail->>'issuer')>0 OR position('|' in detail->>'series')>0 OR position('|' in detail->>'number')>0
   OR finance.reference_date(detail,'invoice_date')>day OR cap<>finance.reference_nonnegative(detail,'eligible_total') OR used<>finance.reference_nonnegative(detail,'paid')
  THEN RETURN false; END IF;
 ELSE RETURN false;
 END CASE;
 RETURN cap>=0 AND used>=0 AND used<=cap;
END $$;
CREATE FUNCTION finance.reference_details_eligible(kind text,detail jsonb,day date)
RETURNS boolean LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
BEGIN
 CASE kind
 WHEN 'CONTRACT' THEN
  RETURN day>=finance.reference_date(detail,'effective_from') AND day<finance.reference_date(detail,'effective_until')
  -- An addendum taking effect after the frozen snapshot requires a new import.
  AND NOT EXISTS(SELECT 1 FROM jsonb_array_elements(detail->'addenda') a WHERE finance.reference_date(a,'effective_on')>finance.reference_date(detail,'frozen_on') AND finance.reference_date(a,'effective_on')<=day);
 WHEN 'BUDGET' THEN RETURN day>=finance.reference_date(detail,'period_start') AND day<finance.reference_date(detail,'period_end');
 WHEN 'ADVANCE' THEN RETURN day>=finance.reference_date(detail,'issued_on');
 WHEN 'INVOICE' THEN RETURN day>=finance.reference_date(detail,'invoice_date');
 ELSE RETURN false;
 END CASE;
END $$;
REVOKE ALL ON FUNCTION finance.reference_nonnegative(jsonb,text),finance.reference_date(jsonb,text),finance.reference_text(jsonb,text),finance.validate_reference_details(text,jsonb,date),finance.reference_details_eligible(text,jsonb,date) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.reference_nonnegative(jsonb,text),finance.reference_date(jsonb,text),finance.reference_text(jsonb,text),finance.validate_reference_details(text,jsonb,date),finance.reference_details_eligible(text,jsonb,date) TO finance_reference_writer,finance_workflow_writer;
-- Immutable financial resource identity and normalized source details. Ledger
-- consumption and coverage can change only via their existing controlled ports.
CREATE FUNCTION finance.reference_resource_identity_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.currency,NEW.cap,NEW.fresh_until,NEW.snapshot_id,NEW.resource_kind,NEW.source_key,NEW.reference_details)
 IS DISTINCT FROM (OLD.tenant_id,OLD.company_id,OLD.id,OLD.currency,OLD.cap,OLD.fresh_until,OLD.snapshot_id,OLD.resource_kind,OLD.source_key,OLD.reference_details)
 THEN RAISE EXCEPTION 'immutable reference resource identity' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER reference_resource_identity_guard BEFORE UPDATE ON finance.financial_resource FOR EACH ROW EXECUTE FUNCTION finance.reference_resource_identity_guard();
