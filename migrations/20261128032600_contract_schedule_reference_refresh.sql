-- Preserve migration 20261127001100 checksum; install split-aware replacements only after split columns exist.
CREATE OR REPLACE FUNCTION finance.prototype_reserve_finance(t uuid,c uuid,i uuid,r uuid,cur text,n numeric,ids uuid[])
RETURNS uuid LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE sorted uuid[]; k uuid; row_resource finance.financial_resource%ROWTYPE;
 prior finance.reservation%ROWTYPE; held numeric;
BEGIN
 IF current_setting('transaction_isolation') <> 'read committed'
 OR t IS NULL OR c IS NULL OR i IS NULL OR r IS NULL OR cur IS NULL
 OR n IS NULL OR n<=0 OR n>=100000000000000000000 OR scale(n)>8
 OR n::text IN ('NaN','Infinity','-Infinity') OR cur !~ '^[A-Z]{3}$'
 OR ids IS NULL OR cardinality(ids) NOT BETWEEN 1 AND 32 OR array_ndims(ids)<>1
 OR array_position(ids,NULL::uuid) IS NOT NULL THEN
  RAISE EXCEPTION 'invalid reservation input' USING ERRCODE='23514';
 END IF;
 SELECT array_agg(DISTINCT x ORDER BY x) INTO sorted FROM unnest(ids) x;
 IF cardinality(sorted)<>cardinality(ids) THEN RAISE EXCEPTION 'duplicate resource' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reservation/v1/'||t::text||'/'||c::text||'/'||i::text,0));
 SELECT * INTO prior FROM finance.reservation WHERE tenant_id=t AND company_id=c AND id=i;
 IF FOUND THEN
  IF prior.revision_id<>r OR prior.currency<>cur OR prior.amount<>n OR prior.resource_ids<>sorted THEN
   RAISE EXCEPTION 'reservation replay mismatch' USING ERRCODE='23505';
  END IF;
  RETURN i;
 END IF;
 FOREACH k IN ARRAY sorted LOOP
  SELECT * INTO row_resource FROM finance.financial_resource
   WHERE tenant_id=t AND company_id=c AND id=k FOR UPDATE;
  IF NOT FOUND OR NOT row_resource.coverage_complete OR row_resource.fresh_until<=clock_timestamp()
   OR row_resource.currency<>cur THEN RAISE EXCEPTION 'unavailable resource' USING ERRCODE='23514'; END IF;
  SELECT COALESCE(sum(coalesce(l.line_remaining,v.remaining)),0) INTO held FROM finance.reservation_line l
   JOIN finance.reservation v ON (v.tenant_id,v.company_id,v.id)=(l.tenant_id,l.company_id,l.reservation_id)
   WHERE l.tenant_id=t AND l.company_id=c AND l.resource_id=k AND v.state IN ('RESERVED','COMMITTED','PARTIAL');
  IF row_resource.cap-row_resource.consumed-held<n THEN
   RAISE EXCEPTION 'cap exceeded' USING ERRCODE='23514';
  END IF;
 END LOOP;
 -- All source locks acquired before any insert. No network or external effects.
 INSERT INTO finance.reservation(tenant_id,company_id,id,revision_id,currency,amount,resource_ids,remaining)
 VALUES(t,c,i,r,cur,n,sorted,n);
 INSERT INTO finance.reservation_line(tenant_id,company_id,reservation_id,resource_id,amount)
 SELECT t,c,i,x,n FROM unnest(sorted) x;
 RETURN i;
END $$;


CREATE OR REPLACE FUNCTION finance.prototype_allocation(t uuid,c uuid,i uuid,r uuid,e uuid,
 operation text,n numeric,digest bytea,original uuid,verified boolean,reconciled boolean)
RETURNS uuid LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE v finance.reservation%ROWTYPE; prior finance.allocation_effect%ROWTYPE;
 source_effect finance.allocation_effect%ROWTYPE; k uuid; reversed numeric; next_state text; next_remaining numeric;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed'
 OR t IS NULL OR c IS NULL OR i IS NULL OR r IS NULL OR e IS NULL
 OR operation IS NULL OR operation NOT IN ('COMMIT','CONSUME','RELEASE','REVERSE')
 OR n IS NULL OR n<0 OR n>=100000000000000000000 OR scale(n)>8
 OR n::text IN ('NaN','Infinity','-Infinity') OR digest IS NULL OR octet_length(digest)<>32
 OR verified IS DISTINCT FROM true OR reconciled IS NULL
 OR ((operation='REVERSE') IS DISTINCT FROM (original IS NOT NULL))
 OR (operation='COMMIT' AND n<>0) OR (operation<>'COMMIT' AND n<=0)
 THEN RAISE EXCEPTION 'invalid allocation command' USING ERRCODE='23514'; END IF;
 -- Lock external event ID, then reservation ID, then resources in canonical order.
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.allocation/v1/'||t::text||'/'||c::text||'/'||e::text,0));
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reservation/v1/'||t::text||'/'||c::text||'/'||i::text,0));
 SELECT * INTO v FROM finance.reservation WHERE tenant_id=t AND company_id=c AND id=i AND revision_id=r FOR UPDATE;
 IF NOT FOUND OR v.split_allocation THEN RAISE EXCEPTION 'unknown or split reservation requires explicit allocation' USING ERRCODE='23514'; END IF;
 SELECT * INTO prior FROM finance.allocation_effect WHERE tenant_id=t AND company_id=c AND id=e;
 IF FOUND THEN
  IF (prior.reservation_id,prior.revision_id,prior.kind,prior.amount,prior.source_digest,prior.reverses)
   IS DISTINCT FROM (i,r,operation,n,digest,original) THEN
   RAISE EXCEPTION 'allocation replay mismatch' USING ERRCODE='23505';
  END IF;
  RETURN e;
 END IF;
 FOREACH k IN ARRAY v.resource_ids LOOP
  PERFORM 1 FROM finance.financial_resource WHERE tenant_id=t AND company_id=c AND id=k FOR UPDATE;
  IF NOT FOUND THEN RAISE EXCEPTION 'missing resource' USING ERRCODE='23514'; END IF;
 END LOOP;
 next_state:=v.state; next_remaining:=v.remaining;
 CASE operation
 WHEN 'COMMIT' THEN
  IF v.state<>'RESERVED' THEN RAISE EXCEPTION 'invalid commit state' USING ERRCODE='23514'; END IF;
  next_state:='COMMITTED';
 WHEN 'CONSUME' THEN
  IF v.state NOT IN ('RESERVED','COMMITTED','PARTIAL') OR n>v.remaining THEN
   RAISE EXCEPTION 'consumption exceeds hold' USING ERRCODE='23514'; END IF;
  next_remaining:=v.remaining-n;
  next_state:=CASE WHEN next_remaining=0 THEN 'CONSUMED' ELSE 'PARTIAL' END;
  UPDATE finance.financial_resource SET consumed=consumed+n WHERE tenant_id=t AND company_id=c AND id=ANY(v.resource_ids);
 WHEN 'RELEASE' THEN
  IF v.state NOT IN ('RESERVED','COMMITTED','PARTIAL') OR n<>v.remaining
   OR (v.state IN ('COMMITTED','PARTIAL') AND NOT reconciled) THEN
   RAISE EXCEPTION 'unreconciled or invalid release' USING ERRCODE='23514'; END IF;
  next_remaining:=0; next_state:='RELEASED';
 WHEN 'REVERSE' THEN
  -- Only verified reversal of a specific original consume event. Never reopen approval/hold.
  SELECT * INTO source_effect FROM finance.allocation_effect
   WHERE tenant_id=t AND company_id=c AND id=original AND reservation_id=i AND revision_id=r AND kind='CONSUME';
  IF NOT FOUND THEN RAISE EXCEPTION 'missing original consumption' USING ERRCODE='23514'; END IF;
  SELECT COALESCE(sum(amount),0) INTO reversed FROM finance.allocation_effect
   WHERE tenant_id=t AND company_id=c AND reverses=original AND kind='REVERSE';
  IF reversed+n>source_effect.amount THEN RAISE EXCEPTION 'reversal exceeds consumption' USING ERRCODE='23514'; END IF;
  UPDATE finance.financial_resource SET consumed=consumed-n WHERE tenant_id=t AND company_id=c AND id=ANY(v.resource_ids);
 END CASE;
 UPDATE finance.reservation SET state=next_state,remaining=next_remaining WHERE tenant_id=t AND company_id=c AND id=i;
 INSERT INTO finance.allocation_effect(tenant_id,company_id,id,reservation_id,revision_id,kind,amount,source_digest,reverses)
 VALUES(t,c,e,i,r,operation,n,digest,original);
 RETURN e;
END $$;
REVOKE ALL ON FUNCTION finance.prototype_allocation(uuid,uuid,uuid,uuid,uuid,text,numeric,bytea,uuid,boolean,boolean) FROM PUBLIC;

-- Contract limits are shared across the full independently approved installment schedule.
CREATE FUNCTION finance.contract_gross_at(detail jsonb,day date)
RETURNS numeric LANGUAGE plpgsql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
DECLARE gross numeric; amendment jsonb;
BEGIN
 gross:=finance.reference_nonnegative(detail,'base_cap');
 FOR amendment IN SELECT value FROM jsonb_array_elements(detail->'addenda') LOOP
  IF finance.reference_date(amendment,'effective_on')<=day AND finance.reference_date(amendment,'approved_on')<=day
  THEN gross:=gross+(amendment->>'delta')::numeric; END IF;
 END LOOP;
 RETURN gross;
END $$;
REVOKE ALL ON FUNCTION finance.contract_gross_at(jsonb,date) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.contract_gross_at(jsonb,date) TO finance_reference_writer,finance_workflow_writer;

CREATE FUNCTION finance.assert_contract_schedule(t uuid,c uuid,contract text)
RETURNS void LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE sample jsonb; gross numeric; total numeric; n integer;
BEGIN
 SELECT x.reference_details INTO sample FROM finance.financial_resource x
 WHERE (x.tenant_id,x.company_id,x.resource_kind)=(t,c,'CONTRACT') AND x.reference_details->>'contract_key'=contract ORDER BY x.id LIMIT 1;
 IF sample IS NULL THEN RAISE EXCEPTION 'contract schedule absent' USING ERRCODE='23514'; END IF;
 gross:=finance.contract_gross_at(sample,finance.reference_date(sample,'frozen_on'));
 SELECT count(*),coalesce(sum(finance.reference_nonnegative(x.reference_details,'installment_gross')),0) INTO n,total
 FROM finance.financial_resource x WHERE (x.tenant_id,x.company_id,x.resource_kind)=(t,c,'CONTRACT') AND x.reference_details->>'contract_key'=contract;
 IF n=0 OR gross<0 OR total>gross OR EXISTS(
 SELECT 1 FROM finance.financial_resource x WHERE (x.tenant_id,x.company_id,x.resource_kind)=(t,c,'CONTRACT') AND x.reference_details->>'contract_key'=contract
 AND (x.reference_details->>'vendor_key',x.reference_details->>'base_cap',x.reference_details->'addenda',x.reference_details->>'effective_from',x.reference_details->>'effective_until',x.reference_details->>'frozen_on',x.reference_details->'acceptance_lines')
 IS DISTINCT FROM (sample->>'vendor_key',sample->>'base_cap',sample->'addenda',sample->>'effective_from',sample->>'effective_until',sample->>'frozen_on',sample->'acceptance_lines'))
 THEN RAISE EXCEPTION 'inconsistent or overallocated contract schedule' USING ERRCODE='23514'; END IF;
 IF (SELECT count(DISTINCT x.currency) FROM finance.financial_resource x WHERE (x.tenant_id,x.company_id,x.resource_kind)=(t,c,'CONTRACT') AND x.reference_details->>'contract_key'=contract)<>1
 THEN RAISE EXCEPTION 'contract currency mismatch' USING ERRCODE='23514'; END IF;
 PERFORM finance.assert_acceptance_refresh(t,c,contract,sample);
END $$;
REVOKE ALL ON FUNCTION finance.assert_contract_schedule(uuid,uuid,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.assert_contract_schedule(uuid,uuid,text) TO finance_reference_writer,finance_workflow_writer;

-- Immutable source snapshots remain historical; stable resource IDs are reused so
-- the local allocation ledger and subsequent reversals cannot be orphaned.
GRANT UPDATE(cap,fresh_until,snapshot_id,reference_details,coverage_complete) ON finance.financial_resource TO finance_reference_writer;
CREATE OR REPLACE FUNCTION finance.reference_resource_identity_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.currency,NEW.resource_kind,NEW.source_key)
 IS DISTINCT FROM (OLD.tenant_id,OLD.company_id,OLD.id,OLD.currency,OLD.resource_kind,OLD.source_key)
 THEN RAISE EXCEPTION 'immutable reference identity' USING ERRCODE='23514'; END IF;
 IF (NEW.cap,NEW.fresh_until,NEW.snapshot_id,NEW.reference_details) IS DISTINCT FROM (OLD.cap,OLD.fresh_until,OLD.snapshot_id,OLD.reference_details) THEN
  IF current_user<>'finance_reference_writer' OR NEW.snapshot_id IS NOT DISTINCT FROM OLD.snapshot_id
  OR NEW.consumed IS DISTINCT FROM OLD.consumed OR NOT NEW.coverage_complete
  OR NOT EXISTS(SELECT 1 FROM finance.reference_snapshot n JOIN finance.reference_snapshot o ON (o.tenant_id,o.company_id,o.id)=(OLD.tenant_id,OLD.company_id,OLD.snapshot_id)
    WHERE (n.tenant_id,n.company_id,n.id)=(NEW.tenant_id,NEW.company_id,NEW.snapshot_id) AND n.state='DRAFT' AND n.source_document_id IS NOT NULL
      AND n.maker_id<>finance.scope_actor() AND n.as_of>o.as_of)
  OR EXISTS(SELECT 1 FROM finance.reservation_line l JOIN finance.reservation r ON (r.tenant_id,r.company_id,r.id)=(l.tenant_id,l.company_id,l.reservation_id)
    WHERE (l.tenant_id,l.company_id,l.resource_id)=(OLD.tenant_id,OLD.company_id,OLD.id) AND r.state IN ('RESERVED','COMMITTED','PARTIAL'))
  THEN RAISE EXCEPTION 'refresh requires newer independently verified balance, no open holds, exact local reconciliation' USING ERRCODE='23514'; END IF;
  IF OLD.resource_kind='CONTRACT' AND (NEW.reference_details->>'contract_key',NEW.reference_details->>'vendor_key') IS DISTINCT FROM (OLD.reference_details->>'contract_key',OLD.reference_details->>'vendor_key') THEN RAISE EXCEPTION 'contract identity changed' USING ERRCODE='23514'; END IF;
  IF OLD.resource_kind='ADVANCE' AND (NEW.reference_details->>'beneficiary_key',NEW.reference_details->>'issued_on') IS DISTINCT FROM (OLD.reference_details->>'beneficiary_key',OLD.reference_details->>'issued_on') THEN RAISE EXCEPTION 'advance identity changed' USING ERRCODE='23514'; END IF;
  IF OLD.resource_kind='BUDGET' AND (NEW.reference_details->>'cost_center',NEW.reference_details->>'account',NEW.reference_details->>'period_start',NEW.reference_details->>'period_end') IS DISTINCT FROM (OLD.reference_details->>'cost_center',OLD.reference_details->>'account',OLD.reference_details->>'period_start',OLD.reference_details->>'period_end') THEN RAISE EXCEPTION 'budget identity changed' USING ERRCODE='23514'; END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE FUNCTION finance.materialize_verified_resource(t uuid,c uuid,s uuid,kind text,item jsonb,day date,fresh timestamptz)
RETURNS void LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE prior finance.financial_resource%ROWTYPE; logical_key text; detail jsonb; new_cap numeric; new_used numeric;
BEGIN
 logical_key:=jsonb_build_array('MANUAL_API',item->>'source_key')::text;
 detail:=(item->'details')||jsonb_build_object('frozen_on',day);
 new_cap:=finance.reference_nonnegative(item,'cap'); new_used:=finance.reference_nonnegative(item,'consumed');
 SELECT * INTO prior FROM finance.financial_resource x WHERE (x.tenant_id,x.company_id,x.resource_kind,x.source_key)=(t,c,kind,logical_key) FOR UPDATE;
 IF FOUND THEN
  IF prior.currency<>item->>'currency' OR prior.consumed<>new_used OR new_cap<new_used THEN RAISE EXCEPTION 'balance mismatch: reconcile external events before refresh' USING ERRCODE='23514'; END IF;
  UPDATE finance.financial_resource x SET cap=new_cap,fresh_until=fresh,snapshot_id=s,reference_details=detail,coverage_complete=true WHERE (x.tenant_id,x.company_id,x.id)=(t,c,prior.id);
 ELSE
  INSERT INTO finance.financial_resource(tenant_id,company_id,id,currency,cap,consumed,coverage_complete,fresh_until,snapshot_id,resource_kind,source_key,reference_details)
  VALUES(t,c,gen_random_uuid(),item->>'currency',new_cap,new_used,true,fresh,s,kind,logical_key,detail);
 END IF;
END $$;
REVOKE ALL ON FUNCTION finance.materialize_verified_resource(uuid,uuid,uuid,text,jsonb,date,timestamptz) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.materialize_verified_resource(uuid,uuid,uuid,text,jsonb,date,timestamptz) TO finance_reference_writer;
