-- E11/E13: independently verified multi-advance allocation, with disjoint
-- per-resource remaining. Existing non-split holds retain their original semantics.
ALTER TABLE finance.reservation ADD COLUMN split_allocation boolean NOT NULL DEFAULT false;
ALTER TABLE finance.reservation_line ADD COLUMN line_remaining numeric(28,8) CHECK(line_remaining IS NULL OR (line_remaining>=0 AND line_remaining<=amount));
ALTER TABLE finance.allocation_effect ADD COLUMN line_allocations jsonb CHECK(line_allocations IS NULL OR jsonb_typeof(line_allocations)='object');
ALTER TABLE finance.payment_ref ADD COLUMN line_allocations jsonb CHECK(line_allocations IS NULL OR jsonb_typeof(line_allocations)='object');
GRANT UPDATE(line_remaining) ON finance.reservation_line TO finance_workflow_writer,finance_reference_writer;
CREATE POLICY split_line_workflow_update ON finance.reservation_line FOR UPDATE TO finance_workflow_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()) WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY split_line_reference_update ON finance.reservation_line FOR UPDATE TO finance_reference_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()) WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE FUNCTION finance.split_reservation_identity_guard() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF OLD.split_allocation IS DISTINCT FROM NEW.split_allocation THEN RAISE EXCEPTION 'immutable split mode' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER split_reservation_identity_guard BEFORE UPDATE ON finance.reservation FOR EACH ROW EXECUTE FUNCTION finance.split_reservation_identity_guard();

CREATE FUNCTION finance.reserve_bound_material(t uuid,c uuid,i uuid,rev uuid,mat jsonb,ids uuid[])
RETURNS uuid LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE n numeric; cur text; k uuid; line numeric; held numeric; fr finance.financial_resource%ROWTYPE; entry jsonb; total numeric:=0;
BEGIN
 n:=(mat->>'amount')::numeric; cur:=mat->>'currency';
 IF mat->>'split' IS DISTINCT FROM 'true' THEN RETURN finance.prototype_reserve_finance(t,c,i,rev,cur,n,ids); END IF;
 IF current_setting('transaction_isolation')<>'read committed' OR n<=0 OR scale(n)>8 OR cardinality(ids) NOT BETWEEN 1 AND 32
 OR NOT EXISTS(SELECT 1 FROM finance.dossier_revision dr JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(dr.tenant_id,dr.company_id,dr.dossier_id) WHERE (dr.tenant_id,dr.company_id,dr.id)=(t,c,rev) AND ds.kind='SETTLEMENT')
 THEN RAISE EXCEPTION 'split requires current settlement' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reservation/v1/'||t::text||'/'||c::text||'/'||i::text,0));
 FOR k IN SELECT DISTINCT z FROM unnest(ids) z ORDER BY z LOOP
  SELECT * INTO fr FROM finance.financial_resource x WHERE (x.tenant_id,x.company_id,x.id)=(t,c,k) FOR UPDATE;
  IF NOT FOUND OR fr.currency<>cur OR NOT fr.coverage_complete OR fr.fresh_until<=clock_timestamp() THEN RAISE EXCEPTION 'unavailable split resource' USING ERRCODE='23514'; END IF;
  SELECT x INTO entry FROM jsonb_array_elements(mat->'resources') x WHERE x->>'id'=k::text;
  line:=(entry->>'allocated')::numeric;
  IF line IS NULL OR line<=0 OR scale(line)>8 OR line>n OR (fr.resource_kind<>'ADVANCE' AND line<>n) THEN RAISE EXCEPTION 'invalid split amount' USING ERRCODE='23514'; END IF;
  IF fr.resource_kind='ADVANCE' THEN total:=total+line; END IF;
  SELECT coalesce(sum(coalesce(l.line_remaining,r.remaining)),0) INTO held FROM finance.reservation_line l JOIN finance.reservation r ON (r.tenant_id,r.company_id,r.id)=(l.tenant_id,l.company_id,l.reservation_id)
  WHERE (l.tenant_id,l.company_id,l.resource_id)=(t,c,k) AND r.state IN ('RESERVED','COMMITTED','PARTIAL');
  IF fr.cap-fr.consumed-held<line THEN RAISE EXCEPTION 'split cap exceeded' USING ERRCODE='23514'; END IF;
 END LOOP;
 IF total<>n THEN RAISE EXCEPTION 'advance allocations must sum to verified applied amount' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.reservation(tenant_id,company_id,id,revision_id,currency,amount,remaining,resource_ids,split_allocation)
 VALUES(t,c,i,rev,cur,n,n,ids,true);
 INSERT INTO finance.reservation_line(tenant_id,company_id,reservation_id,resource_id,amount,line_remaining)
 SELECT t,c,i,(x->>'id')::uuid,(x->>'allocated')::numeric,(x->>'allocated')::numeric FROM jsonb_array_elements(mat->'resources') x;
 RETURN i;
END $$;
ALTER FUNCTION finance.reserve_bound_material(uuid,uuid,uuid,uuid,jsonb,uuid[]) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.reserve_bound_material(uuid,uuid,uuid,uuid,jsonb,uuid[]) FROM PUBLIC;

CREATE FUNCTION finance.apply_split_allocation(t uuid,c uuid,i uuid,rev uuid,e uuid,op text,n numeric,digest bytea,original uuid,allocation jsonb)
RETURNS uuid LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE hold finance.reservation%ROWTYPE; line finance.reservation_line%ROWTYPE; fr finance.financial_resource%ROWTYPE; prior finance.allocation_effect%ROWTYPE; initial finance.allocation_effect%ROWTYPE; k uuid; applied numeric; total numeric:=0; reversed numeric; remain numeric;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' OR op NOT IN ('CONSUME','REVERSE') OR n IS NULL OR n<=0 OR scale(n)>8 OR octet_length(digest)<>32
 OR ((op='REVERSE') IS DISTINCT FROM (original IS NOT NULL)) OR jsonb_typeof(allocation) IS DISTINCT FROM 'object'
 THEN RAISE EXCEPTION 'invalid split observation' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.allocation/v1/'||t::text||'/'||c::text||'/'||e::text,0));
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reservation/v1/'||t::text||'/'||c::text||'/'||i::text,0));
 SELECT * INTO hold FROM finance.reservation x WHERE (x.tenant_id,x.company_id,x.id,x.revision_id)=(t,c,i,rev) FOR UPDATE;
 IF NOT FOUND OR NOT hold.split_allocation THEN RAISE EXCEPTION 'split hold required' USING ERRCODE='23514'; END IF;
 SELECT * INTO prior FROM finance.allocation_effect x WHERE (x.tenant_id,x.company_id,x.id)=(t,c,e);
 IF FOUND THEN
  IF (prior.reservation_id,prior.revision_id,prior.kind,prior.amount,prior.source_digest,prior.reverses,prior.line_allocations) IS DISTINCT FROM (i,rev,op,n,digest,original,allocation)
  THEN RAISE EXCEPTION 'split observation replay mismatch' USING ERRCODE='23505'; END IF;
  RETURN e;
 END IF;
 IF (SELECT count(*) FROM jsonb_object_keys(allocation))<>cardinality(hold.resource_ids) THEN RAISE EXCEPTION 'allocation must explicitly enumerate every held resource' USING ERRCODE='23514'; END IF;
 IF op='CONSUME' AND (hold.state NOT IN ('RESERVED','COMMITTED','PARTIAL') OR n>hold.remaining) THEN RAISE EXCEPTION 'split consumption exceeds hold' USING ERRCODE='23514'; END IF;
 IF op='REVERSE' THEN
  SELECT * INTO initial FROM finance.allocation_effect x WHERE (x.tenant_id,x.company_id,x.id,x.reservation_id,x.revision_id,x.kind)=(t,c,original,i,rev,'CONSUME');
  IF NOT FOUND OR initial.line_allocations IS NULL THEN RAISE EXCEPTION 'original split effect missing' USING ERRCODE='23514'; END IF;
 END IF;
 FOR k IN SELECT z FROM unnest(hold.resource_ids) z ORDER BY z LOOP
  SELECT * INTO STRICT fr FROM finance.financial_resource x WHERE (x.tenant_id,x.company_id,x.id)=(t,c,k) FOR UPDATE;
  SELECT * INTO STRICT line FROM finance.reservation_line x WHERE (x.tenant_id,x.company_id,x.reservation_id,x.resource_id)=(t,c,i,k) FOR UPDATE;
  IF jsonb_typeof(allocation->k::text) IS DISTINCT FROM 'string' OR allocation->>k::text !~ '^[0-9]{1,20}([.][0-9]{1,8})?$' THEN RAISE EXCEPTION 'missing split amount' USING ERRCODE='23514'; END IF;
  applied:=(allocation->>k::text)::numeric;
  IF applied>n OR (fr.resource_kind<>'ADVANCE' AND applied<>n) OR line.line_remaining IS NULL THEN RAISE EXCEPTION 'split dimension mismatch' USING ERRCODE='23514'; END IF;
  IF fr.resource_kind='ADVANCE' THEN total:=total+applied; END IF;
  IF op='CONSUME' THEN
   IF applied>line.line_remaining THEN RAISE EXCEPTION 'split line exhausted' USING ERRCODE='23514'; END IF;
   UPDATE finance.reservation_line x SET line_remaining=line.line_remaining-applied WHERE (x.tenant_id,x.company_id,x.reservation_id,x.resource_id)=(t,c,i,k);
   UPDATE finance.financial_resource x SET consumed=x.consumed+applied WHERE (x.tenant_id,x.company_id,x.id)=(t,c,k);
  ELSE
   SELECT coalesce(sum((x.line_allocations->>k::text)::numeric),0) INTO reversed FROM finance.allocation_effect x WHERE (x.tenant_id,x.company_id,x.reverses,x.kind)=(t,c,original,'REVERSE');
   IF reversed+applied>(initial.line_allocations->>k::text)::numeric OR applied>fr.consumed THEN RAISE EXCEPTION 'split reversal exceeds original' USING ERRCODE='23514'; END IF;
   UPDATE finance.financial_resource x SET consumed=x.consumed-applied WHERE (x.tenant_id,x.company_id,x.id)=(t,c,k);
  END IF;
 END LOOP;
 IF total<>n THEN RAISE EXCEPTION 'split advance amounts do not sum to event' USING ERRCODE='23514'; END IF;
 IF op='CONSUME' THEN
  remain:=hold.remaining-n;
  UPDATE finance.reservation x SET remaining=remain,state=CASE WHEN remain=0 THEN 'CONSUMED' ELSE 'PARTIAL' END WHERE (x.tenant_id,x.company_id,x.id)=(t,c,i);
 END IF;
 INSERT INTO finance.allocation_effect(tenant_id,company_id,id,reservation_id,revision_id,kind,amount,source_digest,reverses,line_allocations)
 VALUES(t,c,e,i,rev,op,n,digest,original,allocation);
 RETURN e;
END $$;
ALTER FUNCTION finance.apply_split_allocation(uuid,uuid,uuid,uuid,uuid,text,numeric,bytea,uuid,jsonb) OWNER TO finance_reference_writer;
REVOKE ALL ON FUNCTION finance.apply_split_allocation(uuid,uuid,uuid,uuid,uuid,text,numeric,bytea,uuid,jsonb) FROM PUBLIC;
