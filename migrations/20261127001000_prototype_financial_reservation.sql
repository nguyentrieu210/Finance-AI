-- E11 isolated synthetic transaction kernel, owner/test only; no application grants.
-- Not a verified reference import, actual payment ledger or production reservation API.
CREATE TABLE finance.financial_resource (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 currency text NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),
 cap numeric(28,8) NOT NULL CHECK(cap>=0 AND cap<100000000000000000000),
 consumed numeric(28,8) NOT NULL CHECK(consumed>=0 AND consumed<=cap),
 coverage_complete boolean NOT NULL,
 fresh_until timestamptz NOT NULL,
 PRIMARY KEY(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id)
);
CREATE TABLE finance.reservation (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 revision_id uuid NOT NULL, currency text NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),
 amount numeric(28,8) NOT NULL CHECK(amount>0),
 resource_ids uuid[] NOT NULL CHECK(cardinality(resource_ids) BETWEEN 1 AND 32),
 state text NOT NULL DEFAULT 'RESERVED' CHECK(state IN ('RESERVED','COMMITTED','RELEASED')),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id)
);
CREATE TABLE finance.reservation_line (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, reservation_id uuid NOT NULL, resource_id uuid NOT NULL,
 amount numeric(28,8) NOT NULL CHECK(amount>0),
 PRIMARY KEY(tenant_id,company_id,reservation_id,resource_id),
 FOREIGN KEY(tenant_id,company_id,reservation_id) REFERENCES finance.reservation(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,resource_id) REFERENCES finance.financial_resource(tenant_id,company_id,id)
);
CREATE INDEX reservation_resource_idx ON finance.reservation_line(tenant_id,company_id,resource_id);
ALTER TABLE finance.financial_resource ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.financial_resource FORCE ROW LEVEL SECURITY;
ALTER TABLE finance.reservation ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.reservation FORCE ROW LEVEL SECURITY;
ALTER TABLE finance.reservation_line ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.reservation_line FORCE ROW LEVEL SECURITY;

CREATE FUNCTION finance.prototype_reserve_finance(t uuid,c uuid,i uuid,r uuid,cur text,n numeric,ids uuid[])
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
  SELECT COALESCE(sum(l.amount),0) INTO held FROM finance.reservation_line l
   JOIN finance.reservation v ON (v.tenant_id,v.company_id,v.id)=(l.tenant_id,l.company_id,l.reservation_id)
   WHERE l.tenant_id=t AND l.company_id=c AND l.resource_id=k AND v.state IN ('RESERVED','COMMITTED');
  IF row_resource.cap-row_resource.consumed-held<n THEN
   RAISE EXCEPTION 'cap exceeded' USING ERRCODE='23514';
  END IF;
 END LOOP;
 -- All source locks acquired before any insert. No network or external effects.
 INSERT INTO finance.reservation(tenant_id,company_id,id,revision_id,currency,amount,resource_ids)
 VALUES(t,c,i,r,cur,n,sorted);
 INSERT INTO finance.reservation_line(tenant_id,company_id,reservation_id,resource_id,amount)
 SELECT t,c,i,x,n FROM unnest(sorted) x;
 RETURN i;
END $$;
REVOKE ALL ON FUNCTION finance.prototype_reserve_finance(uuid,uuid,uuid,uuid,text,numeric,uuid[]) FROM PUBLIC;
-- Release/consume/commit, revision authority, audit, runtime role and reference freshness
-- provenance deliberately not exposed until coordinated transaction gates are implemented.
