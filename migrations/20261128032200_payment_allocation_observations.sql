-- Manual verified external payment event ingestion, no payment dispatch.
ALTER TABLE finance.payment_ref ADD COLUMN reservation_id uuid,ADD COLUMN event_kind text,ADD COLUMN reverses_key text,
 ADD FOREIGN KEY(tenant_id,company_id,reservation_id) REFERENCES finance.reservation(tenant_id,company_id,id),
 ADD CHECK((reservation_id IS NULL AND event_kind IS NULL AND reverses_key IS NULL) OR (reservation_id IS NOT NULL AND event_kind IN ('CONSUME','REVERSE') AND ((event_kind='REVERSE')=(reverses_key IS NOT NULL))));
CREATE UNIQUE INDEX payment_external_event_identity ON finance.payment_ref(tenant_id,company_id,external_key) WHERE reservation_id IS NOT NULL;
GRANT SELECT ON finance.reservation,finance.financial_resource,finance.allocation_effect,finance.dossier_revision,finance.dossier_snapshot TO finance_reference_writer;
GRANT UPDATE(state,remaining) ON finance.reservation TO finance_reference_writer;
GRANT UPDATE(consumed) ON finance.financial_resource TO finance_reference_writer;
GRANT INSERT ON finance.allocation_effect TO finance_reference_writer;
GRANT EXECUTE ON FUNCTION finance.prototype_allocation(uuid,uuid,uuid,uuid,uuid,text,numeric,bytea,uuid,boolean,boolean) TO finance_reference_writer;
CREATE POLICY external_allocation_read ON finance.allocation_effect FOR SELECT TO finance_reference_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY external_allocation_insert ON finance.allocation_effect FOR INSERT TO finance_reference_writer WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND kind IN ('CONSUME','REVERSE'));
CREATE POLICY external_reservation_update ON finance.reservation FOR UPDATE TO finance_reference_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND binding_policy_id IS NOT NULL) WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND binding_policy_id IS NOT NULL);
CREATE POLICY external_resource_update ON finance.financial_resource FOR UPDATE TO finance_reference_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()) WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY external_payment_revision_read ON finance.dossier_revision FOR SELECT TO finance_reference_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE OR REPLACE FUNCTION finance.runtime_record_payment_observation(
 t uuid,c uuid,a uuid,d uuid,v bigint,ref_id uuid,ev_ids uuid[],reason text,idem text,cmd uuid,corr uuid
) RETURNS TABLE(dossier_id uuid,state text,amount text,currency text,as_of text,evidence_ids uuid[],dossier_version bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE
 digest bytea; existing finance.idempotency_record%ROWTYPE; ds finance.dossier%ROWTYPE;
 p finance.payment_ref%ROWTYPE; amt text; asof_txt text; payload jsonb; affected bigint;
 ev_count bigint; hold finance.reservation%ROWTYPE; snap finance.dossier_snapshot%ROWTYPE; source finance.reference_snapshot%ROWTYPE; original uuid; total numeric; payment_state text; source_ev uuid[]; source_row jsonb;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR ref_id IS NULL OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR reason IS NULL OR octet_length(reason) NOT BETWEEN 1 AND 4096 OR btrim(reason)='' OR reason ~ '[[:cntrl:]]'
  OR ev_ids IS NULL OR cardinality(ev_ids) NOT BETWEEN 1 AND 50 OR array_position(ev_ids,NULL::uuid) IS NOT NULL
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid payment observation request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='FINANCE_APPROVER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'finance approver required' USING ERRCODE='42501'; END IF;

 PERFORM pg_advisory_xact_lock(hashtextextended('finance.payment.observe/v2/'||t::text||'/'||c::text||'/'||a::text||'/'||idem,0));
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 digest:=sha256(convert_to('payment.observe:'||d::text||':'||v::text||':'||ref_id::text||':'||reason||':'||array_to_string(ev_ids,','),'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'payment.observe',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>d THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'dossier_id')::uuid,
   existing.result_payload->>'state',
   existing.result_payload->>'amount',
   existing.result_payload->>'currency',
   existing.result_payload->>'as_of',
   ARRAY(SELECT jsonb_array_elements_text(existing.result_payload->'evidence_ids')::uuid),
   (existing.result_payload->>'dossier_version')::bigint;
  RETURN;
 END IF;

 SELECT count(DISTINCT e.id)::bigint INTO ev_count FROM finance.evidence e
  WHERE e.tenant_id=t AND e.company_id=c AND e.id=ANY(ev_ids);
 IF ev_count<>(SELECT count(DISTINCT x) FROM unnest(ev_ids) x) THEN
  RAISE EXCEPTION 'unknown evidence id' USING ERRCODE='22023';
 END IF;

 SELECT ds0.* INTO ds FROM finance.dossier ds0 WHERE (ds0.tenant_id,ds0.company_id,ds0.id)=(t,c,d) FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'dossier not found' USING ERRCODE='42501'; END IF;
 IF ds.owner_id=a OR ds.status<>'ACCEPTED' THEN RAISE EXCEPTION 'independent accepted dossier required' USING ERRCODE='42501'; END IF;
 IF ds.version<>v THEN RAISE EXCEPTION 'dossier version conflict' USING ERRCODE='23505'; END IF;

 SELECT pr.* INTO p FROM finance.payment_ref pr
 JOIN finance.reference_snapshot s ON (s.tenant_id,s.company_id,s.id)=(pr.tenant_id,pr.company_id,pr.snapshot_id)
 WHERE (pr.tenant_id,pr.company_id,pr.id)=(t,c,ref_id) AND pr.reservation_id IS NOT NULL
 AND s.state='APPROVED' AND s.fresh_until>clock_timestamp() AND s.source_document_id IS NOT NULL;
 IF NOT FOUND THEN RAISE EXCEPTION 'verified exact payment event required' USING ERRCODE='23505'; END IF;
 SELECT * INTO STRICT source FROM finance.reference_snapshot s WHERE (s.tenant_id,s.company_id,s.id)=(t,c,p.snapshot_id);
 SELECT item INTO source_row FROM jsonb_array_elements(source.import_payload) item WHERE item->>'source_key'=p.external_key;
 SELECT array_agg(e::uuid ORDER BY e::uuid) INTO source_ev FROM jsonb_array_elements_text(source_row->'evidence_ids') e;
 IF source_ev IS NULL OR NOT (ev_ids <@ source_ev) OR cardinality(ev_ids)<>(SELECT count(DISTINCT e) FROM unnest(ev_ids) e)
 OR NOT EXISTS(SELECT 1 FROM finance.document_blob b WHERE (b.tenant_id,b.company_id,b.id)=(t,c,source.source_document_id) AND b.scan_state='CLEAN' AND b.sha256_digest=source.source_digest)
 THEN RAISE EXCEPTION 'payment evidence not bound to approved source' USING ERRCODE='23514'; END IF;
 SELECT s.* INTO snap FROM finance.dossier_snapshot s JOIN finance.dossier_revision dr ON (dr.tenant_id,dr.company_id,dr.id)=(s.tenant_id,s.company_id,s.revision_id)
 WHERE (s.tenant_id,s.company_id,s.dossier_id,s.reservation_id,dr.number)=(t,c,d,p.reservation_id,ds.revision);
 IF NOT FOUND THEN RAISE EXCEPTION 'payment does not identify sealed reservation' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.allocation/v1/'||t::text||'/'||c::text||'/'||p.id::text,0));
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reservation/v1/'||t::text||'/'||c::text||'/'||p.reservation_id::text,0));
 SELECT x.* INTO hold FROM finance.reservation x WHERE (x.tenant_id,x.company_id,x.id,x.revision_id)=(t,c,p.reservation_id,snap.revision_id) FOR UPDATE;
 IF NOT FOUND OR hold.binding_policy_id IS NULL OR hold.currency<>p.currency THEN RAISE EXCEPTION 'invalid payment reservation' USING ERRCODE='23514'; END IF;
 IF p.event_kind='REVERSE' THEN
  SELECT pr.id INTO original FROM finance.payment_ref pr JOIN finance.allocation_effect ae ON (ae.tenant_id,ae.company_id,ae.id)=(pr.tenant_id,pr.company_id,pr.id)
  WHERE (pr.tenant_id,pr.company_id,pr.external_key,pr.reservation_id,pr.event_kind)=(t,c,p.reverses_key,hold.id,'CONSUME') AND ae.kind='CONSUME' AND pr.currency=p.currency AND pr.occurred_at<=p.occurred_at;
  IF original IS NULL THEN RAISE EXCEPTION 'unobserved original payment' USING ERRCODE='23514'; END IF;
 END IF;
 -- Replays with a different UI idempotency key cannot apply the same bank event twice.
 IF EXISTS(SELECT 1 FROM finance.allocation_effect ae WHERE (ae.tenant_id,ae.company_id,ae.id)=(t,c,p.id))
 THEN RAISE EXCEPTION 'payment already observed' USING ERRCODE='23505'; END IF;
 IF hold.split_allocation THEN
  PERFORM finance.apply_split_allocation(t,c,hold.id,hold.revision_id,p.id,p.event_kind,p.amount,
   sha256(convert_to(jsonb_build_array(encode(source.source_digest,'hex'),p.external_key,p.event_kind,p.amount,p.currency,p.occurred_at,p.reservation_id,p.reverses_key,p.line_allocations)::text,'UTF8')),original,p.line_allocations);
 ELSE
  IF p.line_allocations IS NOT NULL THEN RAISE EXCEPTION 'non-split event cannot provide split allocation' USING ERRCODE='23514'; END IF;
  PERFORM finance.prototype_allocation(t,c,hold.id,hold.revision_id,p.id,p.event_kind,p.amount,
   sha256(convert_to(jsonb_build_array(encode(source.source_digest,'hex'),p.external_key,p.event_kind,p.amount,p.currency,p.occurred_at,p.reservation_id,p.reverses_key)::text,'UTF8')),original,true,false);
 END IF;
 SELECT coalesce(sum(CASE WHEN ae.kind='CONSUME' THEN ae.amount WHEN ae.kind='REVERSE' THEN -ae.amount ELSE 0 END),0)
 INTO total FROM finance.allocation_effect ae WHERE (ae.tenant_id,ae.company_id,ae.reservation_id)=(t,c,hold.id);
 IF total<0 OR total>hold.amount THEN RAISE EXCEPTION 'invalid net payment' USING ERRCODE='23514'; END IF;
 payment_state:=CASE WHEN p.event_kind='REVERSE' AND total=0 THEN 'REVERSED' WHEN p.event_kind='REVERSE' THEN 'PARTIALLY_REVERSED'
 WHEN total=hold.amount THEN 'PAID' ELSE 'PARTIALLY_PAID' END;
 UPDATE finance.dossier d2 SET version=v+1,updated_at=clock_timestamp() WHERE (d2.tenant_id,d2.company_id,d2.id)=(t,c,d) AND d2.version=v;
 GET DIAGNOSTICS affected=ROW_COUNT;
 IF affected<>1 THEN RAISE EXCEPTION 'version conflict' USING ERRCODE='23505'; END IF;

 amt:=trim_scale(total)::text;
 asof_txt:=to_char(p.occurred_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"');
 payload:=jsonb_build_object(
  'dossier_id',d::text,'state',payment_state,'amount',amt,'currency',p.currency,'as_of',asof_txt,
  'evidence_ids',to_jsonb(ev_ids),'dossier_version',v+1,'reference_id',ref_id::text
 );
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,d,'PAYMENT_OBSERVED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,cmd,cmd,d,'PAYMENT_OBSERVED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'payment.observe',idem,digest,d,payload);
 RETURN QUERY SELECT d,payment_state,amt,p.currency,asof_txt,ev_ids,v+1;
END $$;


