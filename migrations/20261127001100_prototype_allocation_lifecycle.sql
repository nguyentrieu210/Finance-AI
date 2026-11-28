-- Synthetic-only E11 lifecycle. Owner-only invoker functions, no runtime grants.
-- No actual ERP observation, no payment API. All callers in this migration are test fixtures.
ALTER TABLE finance.reservation DROP CONSTRAINT reservation_state_check,
 ADD CONSTRAINT reservation_state_check CHECK(state IN ('RESERVED','COMMITTED','PARTIAL','CONSUMED','RELEASED'));
ALTER TABLE finance.reservation ADD COLUMN remaining numeric(28,8);
UPDATE finance.reservation SET remaining=CASE WHEN state='RELEASED' THEN 0 ELSE amount END;
ALTER TABLE finance.reservation ALTER COLUMN remaining SET NOT NULL;
ALTER TABLE finance.reservation ADD CONSTRAINT reservation_remaining_check CHECK(
 remaining>=0 AND remaining<=amount AND
 ((state IN ('RESERVED','COMMITTED') AND remaining=amount) OR
 (state='PARTIAL' AND remaining>0 AND remaining<amount) OR
 (state IN ('CONSUMED','RELEASED') AND remaining=0)));
CREATE TABLE finance.allocation_effect (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 reservation_id uuid NOT NULL, revision_id uuid NOT NULL,
 kind text NOT NULL CHECK(kind IN ('COMMIT','CONSUME','RELEASE','REVERSE')),
 amount numeric(28,8) NOT NULL CHECK(amount>=0),
 source_digest bytea NOT NULL CHECK(octet_length(source_digest)=32),
 reverses uuid, created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,reservation_id) REFERENCES finance.reservation(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,reverses) REFERENCES finance.allocation_effect(tenant_id,company_id,id),
 CHECK((kind='REVERSE')=(reverses IS NOT NULL)),
 CHECK((kind='COMMIT' AND amount=0) OR (kind<>'COMMIT' AND amount>0))
);
CREATE INDEX allocation_reverse_idx ON finance.allocation_effect(tenant_id,company_id,reverses);
ALTER TABLE finance.allocation_effect ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.allocation_effect FORCE ROW LEVEL SECURITY;
CREATE FUNCTION finance.reject_allocation_mutation() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN RAISE EXCEPTION 'immutable allocation effect' USING ERRCODE='23514'; END $$;
CREATE TRIGGER allocation_immutable BEFORE UPDATE OR DELETE ON finance.allocation_effect
 FOR EACH ROW EXECUTE FUNCTION finance.reject_allocation_mutation();
-- Existing reserve signature remains unchanged. Use residual holds, not original line
-- amount, and require the same canonical locks used by the lifecycle commands.
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

CREATE FUNCTION finance.prototype_allocation(t uuid,c uuid,i uuid,r uuid,e uuid,
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
