-- E21: qualify risk snapshot bindings, reauthorize historical receipts, and serialize command keys.
CREATE OR REPLACE FUNCTION finance.runtime_resolve_risk(
 t uuid,c uuid,a uuid,risk uuid,expected bigint,idem_key text,request_hash bytea,
 reason text,evidence uuid[],cmd uuid,corr uuid
) RETURNS bigint LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE prior_hash bytea; prior_result uuid; d uuid; rev uuid; dossier_owner uuid; evidence_count bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
 OR risk IS NULL OR risk='00000000-0000-0000-0000-000000000000'
 OR expected IS NULL OR expected<0 OR expected>9007199254740991
 OR idem_key IS NULL OR octet_length(idem_key) NOT BETWEEN 1 AND 128
 OR request_hash IS NULL OR octet_length(request_hash)<>32
 OR reason IS NULL OR octet_length(reason) NOT BETWEEN 1 AND 4096 OR btrim(reason)=''
 OR reason ~ '[[:cntrl:]]' OR evidence IS NULL OR cardinality(evidence) NOT BETWEEN 1 AND 64
 OR cmd IS NULL OR corr IS NULL
 THEN RAISE EXCEPTION 'invalid risk resolution' USING ERRCODE='22023'; END IF;
 IF (SELECT count(DISTINCT x)::integer FROM unnest(evidence) AS x)<>cardinality(evidence)
 THEN RAISE EXCEPTION 'duplicate risk evidence' USING ERRCODE='22023'; END IF;

 PERFORM pg_advisory_xact_lock(hashtextextended('finance.risk.resolve/v1/'||t::text||'/'||c::text||'/'||risk::text,0));
 IF NOT EXISTS(SELECT 1 FROM finance.risk_signal signal
  WHERE (signal.tenant_id,signal.company_id,signal.id)=(t,c,risk)
    AND finance.risk_assigned_snapshot(t,c,a,signal.snapshot_id))
 THEN RAISE EXCEPTION 'assigned accountant required' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.risk.idempotency/v1/'||t::text||'/'||c::text||'/'||a::text||'/'||idem_key,0));
 SELECT ir.request_hash,ir.result_id INTO prior_hash,prior_result
 FROM finance.idempotency_record ir
 WHERE ir.tenant_id=t AND ir.company_id=c AND ir.actor_id=a AND ir.operation_id='risk.resolve' AND ir.key=idem_key;
 IF FOUND THEN
  IF prior_hash<>$7 OR prior_result<>risk
  THEN RAISE EXCEPTION 'risk replay conflict' USING ERRCODE='23514'; END IF;
  RETURN 1;
 END IF;

 SELECT snap.dossier_id,snap.revision_id,ds.owner_id INTO d,rev,dossier_owner
 FROM finance.risk_signal s
 JOIN finance.dossier_snapshot snap
  ON (snap.tenant_id,snap.company_id,snap.id)=(s.tenant_id,s.company_id,s.snapshot_id)
 JOIN finance.dossier ds
  ON (ds.tenant_id,ds.company_id,ds.id)=(snap.tenant_id,snap.company_id,snap.dossier_id)
 JOIN finance.dossier_revision dr
  ON (dr.tenant_id,dr.company_id,dr.dossier_id,dr.id,dr.number)=
     (ds.tenant_id,ds.company_id,ds.id,snap.revision_id,ds.revision)
 WHERE (s.tenant_id,s.company_id,s.id)=(t,c,risk);
 IF NOT FOUND OR expected<>0 OR dossier_owner=a
 OR EXISTS(SELECT 1 FROM finance.risk_decision rd
   WHERE (rd.tenant_id,rd.company_id,rd.signal_id)=(t,c,risk))
 THEN RAISE EXCEPTION 'risk version conflict' USING ERRCODE='23514'; END IF;

 IF NOT EXISTS(
  SELECT 1 FROM finance.review_assignment ra
  JOIN finance.membership m
   ON (m.tenant_id,m.company_id,m.actor_id)=(ra.tenant_id,ra.company_id,ra.accountant_id)
  WHERE (ra.tenant_id,ra.company_id,ra.dossier_id,ra.revision_id,ra.accountant_id)=(t,c,d,rev,a)
   AND m.role='ACCOUNTANT' AND m.active
   AND m.valid_from<=statement_timestamp()
   AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())
 ) THEN RAISE EXCEPTION 'assigned accountant required' USING ERRCODE='42501'; END IF;

 SELECT count(DISTINCT e.id)::bigint INTO evidence_count
 FROM finance.evidence e
 JOIN finance.ocr_run o
  ON (o.tenant_id,o.company_id,o.id)=(e.tenant_id,e.company_id,e.ocr_run_id)
 JOIN finance.document_ref r
  ON (r.tenant_id,r.company_id,r.id)=(o.tenant_id,o.company_id,o.document_id)
 WHERE e.tenant_id=t AND e.company_id=c AND e.id=ANY(evidence)
  AND r.dossier_id=d AND r.revision_id=rev;
 IF evidence_count<>cardinality(evidence)
 THEN RAISE EXCEPTION 'risk evidence mismatch' USING ERRCODE='22023'; END IF;

 INSERT INTO finance.risk_decision(
  tenant_id,company_id,id,signal_id,actor_id,resolution,reason,evidence_ids
 ) VALUES(t,c,cmd,risk,a,'CONFIRMED',reason,evidence);
 INSERT INTO finance.audit_event(
  tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action
 ) VALUES(t,c,cmd,a,cmd,corr,risk,'RISK_RESOLVED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
 VALUES(t,c,cmd,cmd,risk,'RISK_RESOLVED');
 INSERT INTO finance.idempotency_record(
  tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id
 ) VALUES(t,c,a,'risk.resolve',idem_key,$7,risk);
 RETURN 1;
END $$;
