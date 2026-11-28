-- E21 assigned-accountant risk resolution with evidence-bound immutable decision.
ALTER TABLE finance.risk_decision
 ADD COLUMN evidence_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[],
 ADD CONSTRAINT risk_decision_evidence_bounds CHECK(cardinality(evidence_ids) BETWEEN 1 AND 64);

CREATE ROLE finance_risk_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_risk_writer;
GRANT SELECT ON finance.dossier,finance.dossier_revision,finance.dossier_snapshot,
 finance.review_assignment,finance.membership,finance.risk_signal,finance.risk_decision,
 finance.evidence,finance.ocr_run,finance.document_ref,finance.idempotency_record TO finance_risk_writer;
GRANT INSERT ON finance.risk_decision,finance.audit_event,finance.outbox,finance.idempotency_record TO finance_risk_writer;

CREATE POLICY risk_writer_dossier_read ON finance.dossier FOR SELECT TO finance_risk_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY risk_writer_revision_read ON finance.dossier_revision FOR SELECT TO finance_risk_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY risk_writer_snapshot_read ON finance.dossier_snapshot FOR SELECT TO finance_risk_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY risk_writer_assignment_read ON finance.review_assignment FOR SELECT TO finance_risk_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY risk_writer_membership_read ON finance.membership FOR SELECT TO finance_risk_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY risk_writer_signal_read ON finance.risk_signal FOR SELECT TO finance_risk_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY risk_writer_decision_read ON finance.risk_decision FOR SELECT TO finance_risk_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY risk_writer_decision_insert ON finance.risk_decision FOR INSERT TO finance_risk_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
  AND actor_id=finance.scope_actor());
CREATE POLICY risk_writer_evidence_read ON finance.evidence FOR SELECT TO finance_risk_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY risk_writer_ocr_read ON finance.ocr_run FOR SELECT TO finance_risk_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY risk_writer_document_read ON finance.document_ref FOR SELECT TO finance_risk_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY risk_writer_idempotency_read ON finance.idempotency_record FOR SELECT TO finance_risk_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
  AND actor_id=finance.scope_actor());
CREATE POLICY risk_writer_idempotency_insert ON finance.idempotency_record FOR INSERT TO finance_risk_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
  AND actor_id=finance.scope_actor() AND operation_id='risk.resolve');
CREATE POLICY risk_writer_audit_insert ON finance.audit_event FOR INSERT TO finance_risk_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
  AND actor_id=finance.scope_actor() AND action='RISK_RESOLVED');
CREATE POLICY risk_writer_outbox_insert ON finance.outbox FOR INSERT TO finance_risk_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
  AND event_type='RISK_RESOLVED');

CREATE FUNCTION finance.risk_assigned_snapshot(t uuid,c uuid,a uuid,snapshot uuid)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT t=finance.scope_tenant() AND c=finance.scope_company() AND a=finance.scope_actor()
 AND EXISTS(
  SELECT 1
  FROM finance.dossier_snapshot s
  JOIN finance.dossier d
   ON (d.tenant_id,d.company_id,d.id)=(s.tenant_id,s.company_id,s.dossier_id)
  JOIN finance.review_assignment ra
   ON (ra.tenant_id,ra.company_id,ra.dossier_id,ra.revision_id)=
      (s.tenant_id,s.company_id,s.dossier_id,s.revision_id)
  JOIN finance.membership m
   ON (m.tenant_id,m.company_id,m.actor_id)=(ra.tenant_id,ra.company_id,ra.accountant_id)
  WHERE (s.tenant_id,s.company_id,s.id)=(t,c,snapshot)
   AND ra.accountant_id=a AND d.owner_id<>a
   AND m.role='ACCOUNTANT' AND m.active
   AND m.valid_from<=statement_timestamp()
   AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())
 )
$$;
ALTER FUNCTION finance.risk_assigned_snapshot(uuid,uuid,uuid,uuid) OWNER TO finance_risk_writer;
REVOKE ALL ON FUNCTION finance.risk_assigned_snapshot(uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.risk_assigned_snapshot(uuid,uuid,uuid,uuid) TO finance_runtime;

CREATE POLICY risk_accountant_read ON finance.risk_signal FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND finance.risk_assigned_snapshot(tenant_id,company_id,finance.scope_actor(),snapshot_id)
);
CREATE POLICY risk_decision_accountant_read ON finance.risk_decision FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(
  SELECT 1 FROM finance.risk_signal s
  WHERE (s.tenant_id,s.company_id,s.id)=(risk_decision.tenant_id,risk_decision.company_id,risk_decision.signal_id)
   AND finance.risk_assigned_snapshot(s.tenant_id,s.company_id,finance.scope_actor(),s.snapshot_id)
 )
);

CREATE FUNCTION finance.runtime_resolve_risk(
 t uuid,c uuid,a uuid,risk uuid,expected bigint,idem_key text,request_hash bytea,
 reason text,evidence uuid[],cmd uuid,corr uuid
) RETURNS bigint LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE prior_hash bytea; prior_result uuid; d uuid; rev uuid; owner_id uuid; evidence_count bigint;
BEGIN
 IF t<>finance.scope_tenant() OR c<>finance.scope_company() OR a<>finance.scope_actor()
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
 SELECT request_hash,result_id INTO prior_hash,prior_result
 FROM finance.idempotency_record
 WHERE tenant_id=t AND company_id=c AND actor_id=a AND operation_id='risk.resolve' AND key=idem_key;
 IF FOUND THEN
  IF prior_hash<>request_hash OR prior_result<>risk
  THEN RAISE EXCEPTION 'risk replay conflict' USING ERRCODE='23514'; END IF;
  RETURN 1;
 END IF;

 SELECT ds.dossier_id,ds.revision_id,ds.owner_id INTO d,rev,owner_id
 FROM finance.risk_signal s
 JOIN finance.dossier_snapshot snap
  ON (snap.tenant_id,snap.company_id,snap.id)=(s.tenant_id,s.company_id,s.snapshot_id)
 JOIN finance.dossier ds
  ON (ds.tenant_id,ds.company_id,ds.id)=(snap.tenant_id,snap.company_id,snap.dossier_id)
 WHERE (s.tenant_id,s.company_id,s.id)=(t,c,risk);
 IF NOT FOUND OR expected<>0 OR owner_id=a
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
 ) VALUES(t,c,a,'risk.resolve',idem_key,request_hash,risk);
 RETURN 1;
END $$;
ALTER FUNCTION finance.runtime_resolve_risk(uuid,uuid,uuid,uuid,bigint,text,bytea,text,uuid[],uuid,uuid)
 OWNER TO finance_risk_writer;
REVOKE ALL ON FUNCTION finance.runtime_resolve_risk(uuid,uuid,uuid,uuid,bigint,text,bytea,text,uuid[],uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_resolve_risk(uuid,uuid,uuid,uuid,bigint,text,bytea,text,uuid[],uuid,uuid) TO finance_runtime;

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide','match.confirm','review.begin','review.decide','membership.create','membership.revoke','risk.resolve'));
ALTER TABLE finance.idempotency_record
 ADD COLUMN risk_result_id uuid GENERATED ALWAYS AS
  (CASE WHEN operation_id='risk.resolve' THEN result_id ELSE NULL END) STORED,
 ADD FOREIGN KEY(tenant_id,company_id,risk_result_id)
  REFERENCES finance.risk_signal(tenant_id,company_id,id);

ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED'));
