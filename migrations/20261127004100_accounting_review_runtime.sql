-- E18 accountant review aggregate and E16 critical-field registry.
ALTER TABLE finance.policy_version
 ADD COLUMN criticality_complete boolean NOT NULL DEFAULT false;

CREATE TABLE finance.policy_critical_field (
 tenant_id uuid NOT NULL,
 company_id uuid NOT NULL,
 policy_id uuid NOT NULL,
 clause_id uuid NOT NULL,
 field_path text NOT NULL CHECK(octet_length(field_path) BETWEEN 1 AND 256 AND btrim(field_path)<>''),
 PRIMARY KEY(tenant_id,company_id,policy_id,field_path),
 FOREIGN KEY(tenant_id,company_id,policy_id,clause_id)
  REFERENCES finance.policy_clause(tenant_id,company_id,policy_id,id)
);
ALTER TABLE finance.policy_critical_field ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.policy_critical_field FORCE ROW LEVEL SECURITY;

CREATE TABLE finance.accounting_review (
 tenant_id uuid NOT NULL,
 company_id uuid NOT NULL,
 id uuid NOT NULL,
 dossier_id uuid NOT NULL,
 snapshot_id uuid NOT NULL,
 reviewer_id uuid NOT NULL,
 state text NOT NULL DEFAULT 'PENDING' CHECK(state IN ('PENDING','ACCEPTED','RETURNED')),
 begin_reason text NOT NULL CHECK(octet_length(begin_reason) BETWEEN 1 AND 4096 AND btrim(begin_reason)<>''),
 begin_evidence_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[] CHECK(cardinality(begin_evidence_ids)<=64),
 confirmed_field_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[] CHECK(cardinality(confirmed_field_ids)<=256),
 decision_reason text,
 begin_dossier_version bigint NOT NULL CHECK(begin_dossier_version BETWEEN 1 AND 9007199254740991),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 decided_at timestamptz,
 PRIMARY KEY(tenant_id,company_id,id),
 UNIQUE(tenant_id,company_id,dossier_id,snapshot_id),
 FOREIGN KEY(tenant_id,company_id,dossier_id) REFERENCES finance.dossier(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,snapshot_id) REFERENCES finance.dossier_snapshot(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,reviewer_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK((state='PENDING' AND decision_reason IS NULL AND decided_at IS NULL)
    OR (state IN ('ACCEPTED','RETURNED') AND decision_reason IS NOT NULL AND decided_at IS NOT NULL))
);
ALTER TABLE finance.accounting_review ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.accounting_review FORCE ROW LEVEL SECURITY;

CREATE FUNCTION finance.accounting_review_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'immutable accounting review' USING ERRCODE='23514'; END IF;
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.dossier_id,NEW.snapshot_id,NEW.reviewer_id,
     NEW.begin_reason,NEW.begin_evidence_ids,NEW.begin_dossier_version,NEW.created_at)
    IS DISTINCT FROM
    (OLD.tenant_id,OLD.company_id,OLD.id,OLD.dossier_id,OLD.snapshot_id,OLD.reviewer_id,
     OLD.begin_reason,OLD.begin_evidence_ids,OLD.begin_dossier_version,OLD.created_at)
 THEN RAISE EXCEPTION 'immutable accounting review binding' USING ERRCODE='23514'; END IF;
 IF OLD.state<>'PENDING' OR NEW.state NOT IN ('ACCEPTED','RETURNED')
 OR NEW.decision_reason IS NULL OR btrim(NEW.decision_reason)=''
 OR NEW.decided_at IS NULL
 THEN RAISE EXCEPTION 'invalid accounting review transition' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER accounting_review_immutable
 BEFORE UPDATE OR DELETE ON finance.accounting_review
 FOR EACH ROW EXECUTE FUNCTION finance.accounting_review_guard();

GRANT SELECT ON finance.accounting_review,finance.policy_critical_field,finance.review_assignment,
 finance.field_identity,finance.field_decision,finance.extracted_field,finance.extraction_run,
 finance.ocr_run,finance.document_ref,finance.reconciliation_case,finance.handoff,
 finance.review_decision,finance.critical_confirmation TO finance_workflow_writer;
GRANT INSERT ON finance.accounting_review,finance.review_decision,finance.critical_confirmation
 TO finance_workflow_writer;
GRANT UPDATE(state,confirmed_field_ids,decision_reason,decided_at) ON finance.accounting_review
 TO finance_workflow_writer;

CREATE POLICY workflow_accounting_review_all ON finance.accounting_review TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_policy_critical_read ON finance.policy_critical_field FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_review_assignment_read ON finance.review_assignment FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_field_identity_read ON finance.field_identity FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_field_decision_read ON finance.field_decision FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_extracted_field_read ON finance.extracted_field FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_extraction_run_read ON finance.extraction_run FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_ocr_review_read ON finance.ocr_run FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_document_review_read ON finance.document_ref FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_reconciliation_read ON finance.reconciliation_case FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_review_decision_all ON finance.review_decision TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_critical_confirmation_all ON finance.critical_confirmation TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());

CREATE FUNCTION finance.workflow_begin_accounting_review(
 t uuid,c uuid,d uuid,v bigint,a uuid,review_id uuid,reason text,evidence uuid[],cmd uuid,corr uuid
) RETURNS TABLE(review_id_out uuid,dossier_version_out bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; snap uuid; new_version bigint; sorted uuid[];
BEGIN
 IF t IS NULL OR c IS NULL OR d IS NULL OR a IS NULL OR review_id IS NULL OR cmd IS NULL OR corr IS NULL
 OR v IS NULL OR v<0 OR reason IS NULL OR btrim(reason)='' OR octet_length(reason)>4096
 OR evidence IS NULL OR cardinality(evidence)>64
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR a IS DISTINCT FROM finance.scope_actor()
 THEN RAISE EXCEPTION 'invalid review begin' USING ERRCODE='23514'; END IF;
 SELECT COALESCE(array_agg(DISTINCT x ORDER BY x),ARRAY[]::uuid[]) INTO sorted FROM unnest(evidence) x;
 IF cardinality(sorted)<>cardinality(evidence) OR EXISTS(SELECT 1 FROM unnest(evidence) x WHERE x IS NULL)
 THEN RAISE EXCEPTION 'invalid review evidence' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id=a OR ds.status<>'HANDED_OFF' OR ds.version<>v
 OR NOT EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c
   AND m.actor_id=a AND m.role='ACCOUNTANT' AND m.active
   AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'review authority or state' USING ERRCODE='23514'; END IF;
 SELECT id INTO STRICT rev FROM finance.dossier_revision
  WHERE tenant_id=t AND company_id=c AND dossier_id=d AND number=ds.revision;
 IF NOT EXISTS(SELECT 1 FROM finance.review_assignment ra WHERE ra.tenant_id=t AND ra.company_id=c
   AND ra.dossier_id=d AND ra.revision_id=rev AND ra.accountant_id=a)
 THEN RAISE EXCEPTION 'assigned accountant required' USING ERRCODE='23514'; END IF;
 IF cardinality(sorted)>0 AND (
  SELECT count(DISTINCT e.id) FROM finance.evidence e
  JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(e.tenant_id,e.company_id,e.ocr_run_id)
  JOIN finance.document_ref dr ON (dr.tenant_id,dr.company_id,dr.id)=(o.tenant_id,o.company_id,o.document_id)
  WHERE e.tenant_id=t AND e.company_id=c AND e.id=ANY(sorted)
   AND dr.dossier_id=d AND dr.revision_id=rev
 )<>cardinality(sorted)
 THEN RAISE EXCEPTION 'review evidence outside current revision' USING ERRCODE='23514'; END IF;
 SELECT s.id INTO snap FROM finance.dossier_snapshot s
  WHERE s.tenant_id=t AND s.company_id=c AND s.dossier_id=d AND s.revision_id=rev
  ORDER BY s.sealed_at DESC,s.id DESC LIMIT 1;
 IF snap IS NULL
 OR NOT EXISTS(SELECT 1 FROM finance.handoff h WHERE h.tenant_id=t AND h.company_id=c
    AND h.snapshot_id=snap AND h.state='ACKNOWLEDGED' AND h.external_ref IS NOT NULL)
 OR EXISTS(SELECT 1 FROM finance.reconciliation_case r JOIN finance.handoff h
    ON (h.tenant_id,h.company_id,h.id)=(r.tenant_id,r.company_id,r.handoff_id)
    WHERE h.tenant_id=t AND h.company_id=c AND h.snapshot_id=snap AND r.state='OPEN')
 THEN RAISE EXCEPTION 'authoritative handoff evidence required' USING ERRCODE='23514'; END IF;
 IF EXISTS(SELECT 1 FROM finance.accounting_review r WHERE r.tenant_id=t AND r.company_id=c
    AND r.dossier_id=d AND r.snapshot_id=snap)
 THEN RAISE EXCEPTION 'review already exists' USING ERRCODE='23514'; END IF;
 new_version:=v+1;
 INSERT INTO finance.accounting_review(
  tenant_id,company_id,id,dossier_id,snapshot_id,reviewer_id,begin_reason,begin_evidence_ids,begin_dossier_version
 ) VALUES(t,c,review_id,d,snap,a,reason,sorted,new_version);
 UPDATE finance.dossier SET status='ACCOUNTING_REVIEW',version=new_version,
  updated_at=GREATEST(updated_at,clock_timestamp())
 WHERE tenant_id=t AND company_id=c AND id=d;
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
 VALUES(t,c,cmd,a,cmd,corr,d,'REVIEW_BEGUN');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
 VALUES(t,c,cmd,cmd,d,'REVIEW_BEGUN');
 RETURN QUERY SELECT review_id,new_version;
END $$;
ALTER FUNCTION finance.workflow_begin_accounting_review(uuid,uuid,uuid,bigint,uuid,uuid,text,uuid[],uuid,uuid)
 OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.workflow_begin_accounting_review(uuid,uuid,uuid,bigint,uuid,uuid,text,uuid[],uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.workflow_begin_accounting_review(uuid,uuid,uuid,bigint,uuid,uuid,text,uuid[],uuid,uuid)
 TO finance_runtime;

CREATE FUNCTION finance.workflow_decide_accounting_review(
 t uuid,c uuid,d uuid,v bigint,a uuid,review_id uuid,snap_id uuid,
 decision text,confirmed uuid[],reason text,cmd uuid,corr uuid
) RETURNS bigint
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; r finance.accounting_review%ROWTYPE;
 s finance.dossier_snapshot%ROWTYPE; expected uuid[]:=ARRAY[]::uuid[]; submitted uuid[]:=ARRAY[]::uuid[];
 critical_count bigint; confirmed_count bigint; new_version bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR d IS NULL OR a IS NULL OR review_id IS NULL OR snap_id IS NULL
 OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0 OR decision NOT IN ('ACCEPTED','RETURNED')
 OR confirmed IS NULL OR cardinality(confirmed)>256 OR reason IS NULL OR btrim(reason)='' OR octet_length(reason)>4096
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR a IS DISTINCT FROM finance.scope_actor()
 THEN RAISE EXCEPTION 'invalid review decision' USING ERRCODE='23514'; END IF;
 SELECT COALESCE(array_agg(DISTINCT x ORDER BY x),ARRAY[]::uuid[]) INTO submitted FROM unnest(confirmed) x;
 IF cardinality(submitted)<>cardinality(confirmed) OR EXISTS(SELECT 1 FROM unnest(confirmed) x WHERE x IS NULL)
 THEN RAISE EXCEPTION 'invalid confirmed fields' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id=a OR ds.status<>'ACCOUNTING_REVIEW' OR ds.version<>v
 OR NOT EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c
   AND m.actor_id=a AND m.role='ACCOUNTANT' AND m.active
   AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'review decision authority or state' USING ERRCODE='23514'; END IF;
 SELECT id INTO STRICT rev FROM finance.dossier_revision
  WHERE tenant_id=t AND company_id=c AND dossier_id=d AND number=ds.revision;
 IF NOT EXISTS(SELECT 1 FROM finance.review_assignment ra WHERE ra.tenant_id=t AND ra.company_id=c
   AND ra.dossier_id=d AND ra.revision_id=rev AND ra.accountant_id=a)
 THEN RAISE EXCEPTION 'assigned accountant required' USING ERRCODE='23514'; END IF;
 SELECT * INTO r FROM finance.accounting_review WHERE tenant_id=t AND company_id=c AND id=review_id
   AND dossier_id=d AND snapshot_id=snap_id AND reviewer_id=a AND state='PENDING' FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'pending review missing' USING ERRCODE='23514'; END IF;
 SELECT * INTO s FROM finance.dossier_snapshot WHERE tenant_id=t AND company_id=c AND id=snap_id
   AND dossier_id=d AND revision_id=rev;
 IF NOT FOUND THEN RAISE EXCEPTION 'stale review snapshot' USING ERRCODE='23514'; END IF;
 IF EXISTS(SELECT 1 FROM finance.reconciliation_case rc JOIN finance.handoff h
    ON (h.tenant_id,h.company_id,h.id)=(rc.tenant_id,rc.company_id,rc.handoff_id)
    WHERE h.tenant_id=t AND h.company_id=c AND h.snapshot_id=snap_id AND rc.state='OPEN')
 THEN RAISE EXCEPTION 'reconciliation remains open' USING ERRCODE='23514'; END IF;
 IF decision='ACCEPTED' THEN
  IF NOT EXISTS(SELECT 1 FROM finance.policy_version p WHERE p.tenant_id=t AND p.company_id=c
      AND p.id=s.policy_id AND p.state='PUBLISHED' AND p.criticality_complete)
  OR NOT EXISTS(SELECT 1 FROM finance.handoff h WHERE h.tenant_id=t AND h.company_id=c
      AND h.snapshot_id=snap_id AND h.state='ACKNOWLEDGED' AND h.external_ref IS NOT NULL)
  OR s.check_run_id IS NULL
  OR EXISTS(SELECT 1 FROM finance.check_result x WHERE x.tenant_id=t AND x.company_id=c
      AND x.run_id=s.check_run_id AND x.severity='BLOCK' AND x.outcome NOT IN ('PASS','NOT_APPLICABLE'))
  THEN RAISE EXCEPTION 'acceptance guards failed' USING ERRCODE='23514'; END IF;

  SELECT count(*) INTO critical_count FROM finance.policy_critical_field pc
   WHERE pc.tenant_id=t AND pc.company_id=c AND pc.policy_id=s.policy_id;

  WITH valid AS (
   SELECT pc.field_path,fi.id,fi.evidence_ids
   FROM finance.policy_critical_field pc
   JOIN LATERAL (
    SELECT fi0.id,fd0.evidence_ids
    FROM finance.document_ref dr
    JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.document_id,o.revision_id)=
      (dr.tenant_id,dr.company_id,dr.id,dr.revision_id)
    JOIN finance.extraction_run er ON (er.tenant_id,er.company_id,er.ocr_run_id)=(o.tenant_id,o.company_id,o.id)
    JOIN finance.extracted_field ef ON (ef.tenant_id,ef.company_id,ef.run_id)=(er.tenant_id,er.company_id,er.id)
    JOIN finance.field_identity fi0 ON (fi0.tenant_id,fi0.company_id,fi0.run_id,fi0.field_path)=
      (ef.tenant_id,ef.company_id,ef.run_id,ef.field_path)
    JOIN LATERAL (
     SELECT fd1.evidence_ids
     FROM finance.field_decision fd1
     WHERE fd1.tenant_id=ef.tenant_id AND fd1.company_id=ef.company_id AND fd1.run_id=ef.run_id
      AND fd1.field_path=ef.field_path AND fd1.dossier_id=d AND fd1.revision_id=rev
      AND fd1.actor_id=a AND fd1.disposition IN ('CONFIRMED_CANDIDATE','CORRECTED')
      AND cardinality(fd1.evidence_ids)>0
     ORDER BY fd1.dossier_version DESC,fd1.created_at DESC,fd1.id DESC LIMIT 1
    ) fd0 ON true
    WHERE dr.tenant_id=t AND dr.company_id=c AND dr.dossier_id=d AND dr.revision_id=rev
     AND ef.field_path=pc.field_path
     AND NOT EXISTS(SELECT 1 FROM finance.ocr_run newer WHERE newer.tenant_id=o.tenant_id
       AND newer.company_id=o.company_id AND newer.document_id=o.document_id AND newer.revision_id=o.revision_id
       AND (newer.created_at>o.created_at OR (newer.created_at=o.created_at AND newer.id>o.id)))
    ORDER BY o.created_at DESC,o.id DESC LIMIT 1
   ) fi ON true
   WHERE pc.tenant_id=t AND pc.company_id=c AND pc.policy_id=s.policy_id
  )
  SELECT count(*),COALESCE(array_agg(id ORDER BY id),ARRAY[]::uuid[])
   INTO confirmed_count,expected FROM valid;
  IF confirmed_count<>critical_count OR submitted IS DISTINCT FROM expected
  THEN RAISE EXCEPTION 'critical field confirmation incomplete' USING ERRCODE='23514'; END IF;

  INSERT INTO finance.critical_confirmation(
   tenant_id,company_id,snapshot_id,field_path,actor_id,evidence_id,decision_version
  )
  SELECT t,c,snap_id,pc.field_path,a,fd.evidence_ids[1],v
  FROM finance.policy_critical_field pc
  JOIN finance.field_identity fi ON fi.tenant_id=t AND fi.company_id=c AND fi.id=ANY(submitted)
  JOIN finance.extracted_field ef ON (ef.tenant_id,ef.company_id,ef.run_id,ef.field_path)=
    (fi.tenant_id,fi.company_id,fi.run_id,fi.field_path)
  JOIN LATERAL (
   SELECT x.evidence_ids FROM finance.field_decision x
   WHERE x.tenant_id=fi.tenant_id AND x.company_id=fi.company_id AND x.run_id=fi.run_id
    AND x.field_path=fi.field_path AND x.dossier_id=d AND x.revision_id=rev AND x.actor_id=a
    AND x.disposition IN ('CONFIRMED_CANDIDATE','CORRECTED') AND cardinality(x.evidence_ids)>0
   ORDER BY x.dossier_version DESC,x.created_at DESC,x.id DESC LIMIT 1
  ) fd ON true
  WHERE pc.tenant_id=t AND pc.company_id=c AND pc.policy_id=s.policy_id AND pc.field_path=ef.field_path
  ON CONFLICT DO NOTHING;
 END IF;

 UPDATE finance.accounting_review SET state=decision,confirmed_field_ids=submitted,
  decision_reason=reason,decided_at=clock_timestamp()
 WHERE tenant_id=t AND company_id=c AND id=review_id;
 INSERT INTO finance.review_decision(tenant_id,company_id,id,snapshot_id,actor_id,decision,reason)
 VALUES(t,c,cmd,snap_id,a,decision,reason);
 new_version:=v+1;
 UPDATE finance.dossier SET status=decision,version=new_version,updated_at=GREATEST(updated_at,clock_timestamp())
 WHERE tenant_id=t AND company_id=c AND id=d;
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
 VALUES(t,c,cmd,a,cmd,corr,d,'REVIEW_DECIDED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
 VALUES(t,c,cmd,cmd,d,'REVIEW_DECIDED');
 RETURN new_version;
END $$;
ALTER FUNCTION finance.workflow_decide_accounting_review(uuid,uuid,uuid,bigint,uuid,uuid,uuid,text,uuid[],text,uuid,uuid)
 OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.workflow_decide_accounting_review(uuid,uuid,uuid,bigint,uuid,uuid,uuid,text,uuid[],text,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.workflow_decide_accounting_review(uuid,uuid,uuid,bigint,uuid,uuid,uuid,text,uuid[],text,uuid,uuid)
 TO finance_runtime;

CREATE FUNCTION finance.runtime_review_get(t uuid,c uuid,d uuid,a uuid)
RETURNS TABLE(
 id uuid,snapshot_id uuid,reviewer_id uuid,decision text,confirmed_field_ids uuid[],reason text,dossier_version bigint
)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT r.id,r.snapshot_id,r.reviewer_id,r.state,r.confirmed_field_ids,
  CASE WHEN r.state='PENDING' THEN r.begin_reason ELSE r.decision_reason END,
  ds.version
 FROM finance.accounting_review r
 JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(r.tenant_id,r.company_id,r.dossier_id)
 WHERE t=finance.scope_tenant() AND c=finance.scope_company() AND a=finance.scope_actor()
 AND r.tenant_id=t AND r.company_id=c AND r.dossier_id=d
 AND (
  (ds.owner_id=a AND EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c
    AND m.actor_id=a AND m.role='REQUESTER' AND m.active
    AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())))
  OR
  (r.reviewer_id=a AND EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c
    AND m.actor_id=a AND m.role='ACCOUNTANT' AND m.active
    AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())))
 )
 ORDER BY r.created_at DESC,r.id DESC LIMIT 1
$$;
ALTER FUNCTION finance.runtime_review_get(uuid,uuid,uuid,uuid) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.runtime_review_get(uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_review_get(uuid,uuid,uuid,uuid) TO finance_runtime;

CREATE FUNCTION finance.runtime_review_dossier(t uuid,c uuid,d uuid,a uuid)
RETURNS TABLE(
 id uuid,company_id uuid,code text,kind text,status text,revision bigint,version bigint,owner_id uuid,updated_at text
)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT ds.id,ds.company_id,ds.code,ds.kind,ds.status,ds.revision,ds.version,ds.owner_id,
  to_char(ds.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')
 FROM finance.dossier ds
 JOIN finance.dossier_revision dr
  ON (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number)=(ds.tenant_id,ds.company_id,ds.id,ds.revision)
 JOIN finance.review_assignment ra
  ON (ra.tenant_id,ra.company_id,ra.dossier_id,ra.revision_id)=(ds.tenant_id,ds.company_id,ds.id,dr.id)
 WHERE t=finance.scope_tenant() AND c=finance.scope_company() AND a=finance.scope_actor()
 AND ds.tenant_id=t AND ds.company_id=c AND ds.id=d AND ra.accountant_id=a
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c
  AND m.actor_id=a AND m.role='ACCOUNTANT' AND m.active
  AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
$$;
ALTER FUNCTION finance.runtime_review_dossier(uuid,uuid,uuid,uuid) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.runtime_review_dossier(uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_review_dossier(uuid,uuid,uuid,uuid) TO finance_runtime;

CREATE FUNCTION finance.runtime_review_queue(t uuid,c uuid,a uuid,after_id uuid,n integer)
RETURNS TABLE(
 id uuid,company_id uuid,code text,kind text,status text,revision bigint,version bigint,owner_id uuid,updated_at text
)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT ds.id,ds.company_id,ds.code,ds.kind,ds.status,ds.revision,ds.version,ds.owner_id,
  to_char(ds.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')
 FROM finance.dossier ds
 JOIN finance.dossier_revision dr
  ON (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number)=(ds.tenant_id,ds.company_id,ds.id,ds.revision)
 JOIN finance.review_assignment ra
  ON (ra.tenant_id,ra.company_id,ra.dossier_id,ra.revision_id)=(ds.tenant_id,ds.company_id,ds.id,dr.id)
 WHERE t=finance.scope_tenant() AND c=finance.scope_company() AND a=finance.scope_actor()
 AND ds.tenant_id=t AND ds.company_id=c AND ra.accountant_id=a AND ds.id>after_id
 AND ds.status IN ('HANDED_OFF','ACCOUNTING_REVIEW') AND n BETWEEN 1 AND 100
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c
  AND m.actor_id=a AND m.role='ACCOUNTANT' AND m.active
  AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 ORDER BY ds.id LIMIT n
$$;
ALTER FUNCTION finance.runtime_review_queue(uuid,uuid,uuid,uuid,integer) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.runtime_review_queue(uuid,uuid,uuid,uuid,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_review_queue(uuid,uuid,uuid,uuid,integer) TO finance_runtime;

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide','match.confirm','review.begin','review.decide'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED'));
