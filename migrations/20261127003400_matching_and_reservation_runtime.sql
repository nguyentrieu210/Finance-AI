-- E10 manual evidence-pair confirmation and E11 scoped reservation projection.
ALTER TABLE finance.match_decision
 ADD COLUMN dossier_version bigint NOT NULL DEFAULT 0 CHECK(dossier_version BETWEEN 0 AND 9007199254740991);

CREATE ROLE finance_matching_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_matching_writer;
GRANT SELECT ON finance.dossier,finance.dossier_revision,finance.membership,finance.review_assignment,
 finance.document_ref,finance.ocr_run,finance.evidence,finance.match_result,finance.match_decision
 TO finance_matching_writer;
GRANT INSERT ON finance.match_result,finance.match_decision,finance.audit_event,finance.outbox
 TO finance_matching_writer;
GRANT UPDATE(version,updated_at) ON finance.dossier TO finance_matching_writer;

CREATE POLICY matching_dossier_read ON finance.dossier FOR SELECT TO finance_matching_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY matching_dossier_update ON finance.dossier FOR UPDATE TO finance_matching_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY matching_revision_read ON finance.dossier_revision FOR SELECT TO finance_matching_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY matching_membership_read ON finance.membership FOR SELECT TO finance_matching_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY matching_assignment_read ON finance.review_assignment FOR SELECT TO finance_matching_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY matching_document_read ON finance.document_ref FOR SELECT TO finance_matching_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY matching_ocr_read ON finance.ocr_run FOR SELECT TO finance_matching_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY matching_evidence_read ON finance.evidence FOR SELECT TO finance_matching_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY matching_result_read ON finance.match_result FOR SELECT TO finance_matching_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY matching_result_insert ON finance.match_result FOR INSERT TO finance_matching_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY matching_decision_read ON finance.match_decision FOR SELECT TO finance_matching_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY matching_decision_insert ON finance.match_decision FOR INSERT TO finance_matching_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY matching_audit_insert ON finance.audit_event FOR INSERT TO finance_matching_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY matching_outbox_insert ON finance.outbox FOR INSERT TO finance_matching_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());

CREATE FUNCTION finance.runtime_matches(t uuid,c uuid,d uuid,a uuid,after_id uuid,n integer)
RETURNS TABLE(
 id uuid,left_evidence_id uuid,right_evidence_id uuid,state text,reason text
)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF d IS NULL OR after_id IS NULL OR n IS NULL OR n NOT BETWEEN 1 AND 100
 OR NOT finance.field_access_allowed(t,c,d,a)
 THEN RAISE EXCEPTION 'match read forbidden or invalid' USING ERRCODE='42501'; END IF;
 RETURN QUERY
 SELECT m.id,m.left_evidence_id,m.right_evidence_id,
  CASE WHEN q.decision='CONFIRMED' THEN 'CONFIRMED'
       WHEN q.decision='REJECTED' THEN 'UNMATCHED'
       ELSE m.state END,
  COALESCE(q.reason,m.reason)
 FROM finance.match_result m
 LEFT JOIN finance.match_decision q
  ON (q.tenant_id,q.company_id,q.result_id)=(m.tenant_id,m.company_id,m.id)
 JOIN finance.dossier ds
  ON (ds.tenant_id,ds.company_id,ds.id)=(m.tenant_id,m.company_id,m.dossier_id)
 JOIN finance.dossier_revision dr
  ON (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number)=(ds.tenant_id,ds.company_id,ds.id,ds.revision)
 WHERE m.tenant_id=t AND m.company_id=c AND m.dossier_id=d AND m.revision_id=dr.id
  AND m.id>after_id
 ORDER BY m.id LIMIT n;
END $$;
ALTER FUNCTION finance.runtime_matches(uuid,uuid,uuid,uuid,uuid,integer) OWNER TO finance_matching_writer;
REVOKE ALL ON FUNCTION finance.runtime_matches(uuid,uuid,uuid,uuid,uuid,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_matches(uuid,uuid,uuid,uuid,uuid,integer) TO finance_runtime;

CREATE FUNCTION finance.runtime_match_confirm(
 t uuid,c uuid,d uuid,v bigint,a uuid,result_id uuid,decision_id uuid,
 left_evidence uuid,right_evidence uuid,why text,cmd uuid,corr uuid
) RETURNS bigint
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; evidence_count bigint; new_version bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR d IS NULL OR v IS NULL OR v<0 OR a IS NULL
 OR result_id IS NULL OR decision_id IS NULL OR left_evidence IS NULL OR right_evidence IS NULL
 OR left_evidence=right_evidence OR why IS NULL OR octet_length(why) NOT BETWEEN 1 AND 2048
 OR btrim(why)='' OR cmd IS NULL OR corr IS NULL
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR a IS DISTINCT FROM finance.scope_actor()
 THEN RAISE EXCEPTION 'invalid manual match' USING ERRCODE='23514'; END IF;

 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id=a OR ds.version<>v OR ds.status NOT IN ('DRAFT','NEEDS_INPUT','REVIEWABLE')
 THEN RAISE EXCEPTION 'manual match state or self review' USING ERRCODE='23514'; END IF;

 SELECT id INTO STRICT rev FROM finance.dossier_revision
 WHERE tenant_id=t AND company_id=c AND dossier_id=d AND number=ds.revision;
 IF NOT EXISTS(
  SELECT 1 FROM finance.review_assignment ra WHERE ra.tenant_id=t AND ra.company_id=c
   AND ra.dossier_id=d AND ra.revision_id=rev AND ra.accountant_id=a
 ) OR NOT EXISTS(
  SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c
   AND m.actor_id=a AND m.role='ACCOUNTANT' AND m.active
   AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp())
 )
 THEN RAISE EXCEPTION 'assigned accountant required' USING ERRCODE='23514'; END IF;

 SELECT count(DISTINCT e.id) INTO evidence_count
 FROM finance.evidence e
 JOIN finance.ocr_run o
  ON (o.tenant_id,o.company_id,o.id)=(e.tenant_id,e.company_id,e.ocr_run_id)
 JOIN finance.document_ref r
  ON (r.tenant_id,r.company_id,r.id)=(o.tenant_id,o.company_id,o.document_id)
 WHERE e.tenant_id=t AND e.company_id=c
  AND e.id=ANY(ARRAY[left_evidence,right_evidence])
  AND r.dossier_id=d AND r.revision_id=rev AND o.revision_id=rev;
 IF evidence_count<>2 THEN
  RAISE EXCEPTION 'manual match evidence must belong to current revision' USING ERRCODE='23514'; END IF;

 INSERT INTO finance.match_result(
  tenant_id,company_id,id,dossier_id,revision_id,left_evidence_id,right_evidence_id,state,reason
 ) VALUES(t,c,result_id,d,rev,left_evidence,right_evidence,'SUGGESTED',why);
 INSERT INTO finance.match_decision(
  tenant_id,company_id,id,result_id,actor_id,decision,reason,dossier_version
 ) VALUES(t,c,decision_id,result_id,a,'CONFIRMED',why,v+1);

 UPDATE finance.dossier
 SET version=version+1,updated_at=GREATEST(updated_at,clock_timestamp())
 WHERE tenant_id=t AND company_id=c AND id=d
 RETURNING version INTO new_version;

 INSERT INTO finance.audit_event(
  tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action
 ) VALUES(t,c,cmd,a,cmd,corr,d,'MATCH_CONFIRMED');
 INSERT INTO finance.outbox(
  tenant_id,company_id,id,command_id,resource_id,event_type
 ) VALUES(t,c,cmd,cmd,d,'MATCH_CONFIRMED');
 RETURN new_version;
END $$;
ALTER FUNCTION finance.runtime_match_confirm(uuid,uuid,uuid,bigint,uuid,uuid,uuid,uuid,uuid,text,uuid,uuid)
 OWNER TO finance_matching_writer;
REVOKE ALL ON FUNCTION finance.runtime_match_confirm(uuid,uuid,uuid,bigint,uuid,uuid,uuid,uuid,uuid,text,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_match_confirm(uuid,uuid,uuid,bigint,uuid,uuid,uuid,uuid,uuid,text,uuid,uuid)
 TO finance_runtime;

CREATE FUNCTION finance.runtime_match_receipt(t uuid,c uuid,d uuid,a uuid,result_id uuid)
RETURNS TABLE(
 id uuid,left_evidence_id uuid,right_evidence_id uuid,state text,reason text,dossier_version bigint
)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF result_id IS NULL OR NOT finance.field_access_allowed(t,c,d,a)
 THEN RAISE EXCEPTION 'match receipt forbidden' USING ERRCODE='42501'; END IF;
 RETURN QUERY
 SELECT m.id,m.left_evidence_id,m.right_evidence_id,
  CASE q.decision WHEN 'CONFIRMED' THEN 'CONFIRMED' ELSE 'UNMATCHED' END,
  q.reason,q.dossier_version
 FROM finance.match_result m
 JOIN finance.match_decision q
  ON (q.tenant_id,q.company_id,q.result_id)=(m.tenant_id,m.company_id,m.id)
 WHERE m.tenant_id=t AND m.company_id=c AND m.dossier_id=d AND m.id=result_id AND q.actor_id=a
 LIMIT 1;
END $$;
ALTER FUNCTION finance.runtime_match_receipt(uuid,uuid,uuid,uuid,uuid) OWNER TO finance_matching_writer;
REVOKE ALL ON FUNCTION finance.runtime_match_receipt(uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_match_receipt(uuid,uuid,uuid,uuid,uuid) TO finance_runtime;

-- E11 read projection. Reservation mutations remain internal transaction ports.
CREATE ROLE finance_reservation_reader NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_reservation_reader;
GRANT SELECT ON finance.dossier,finance.dossier_revision,finance.dossier_snapshot,
 finance.reservation,finance.reservation_line,finance.allocation_effect
 TO finance_reservation_reader;
CREATE POLICY reservation_reader_dossier ON finance.dossier FOR SELECT TO finance_reservation_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY reservation_reader_revision ON finance.dossier_revision FOR SELECT TO finance_reservation_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY reservation_reader_snapshot ON finance.dossier_snapshot FOR SELECT TO finance_reservation_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY reservation_reader_reservation ON finance.reservation FOR SELECT TO finance_reservation_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY reservation_reader_line ON finance.reservation_line FOR SELECT TO finance_reservation_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY reservation_reader_effect ON finance.allocation_effect FOR SELECT TO finance_reservation_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());

CREATE FUNCTION finance.runtime_reservations(t uuid,c uuid,d uuid,a uuid,after_id uuid,n integer)
RETURNS TABLE(
 id uuid,snapshot_id uuid,resource_id uuid,amount text,remaining text,currency text,state text,version bigint
)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF d IS NULL OR after_id IS NULL OR n IS NULL OR n NOT BETWEEN 1 AND 100
 OR NOT finance.field_access_allowed(t,c,d,a)
 THEN RAISE EXCEPTION 'reservation read forbidden or invalid' USING ERRCODE='42501'; END IF;
 RETURN QUERY
 SELECT r.id,s.id,l.resource_id,r.amount::text,r.remaining::text,r.currency,r.state,
  (SELECT count(*)::bigint FROM finance.allocation_effect e
    WHERE (e.tenant_id,e.company_id,e.reservation_id)=(r.tenant_id,r.company_id,r.id))
 FROM finance.reservation r
 JOIN finance.reservation_line l
  ON (l.tenant_id,l.company_id,l.reservation_id)=(r.tenant_id,r.company_id,r.id)
 JOIN finance.dossier_revision dr
  ON (dr.tenant_id,dr.company_id,dr.id)=(r.tenant_id,r.company_id,r.revision_id)
 JOIN finance.dossier ds
  ON (ds.tenant_id,ds.company_id,ds.id)=(dr.tenant_id,dr.company_id,dr.dossier_id)
 JOIN LATERAL (
  SELECT s1.id FROM finance.dossier_snapshot s1
  WHERE s1.tenant_id=r.tenant_id AND s1.company_id=r.company_id AND s1.dossier_id=d
   AND s1.revision_id=r.revision_id
  ORDER BY s1.sealed_at DESC,s1.id DESC LIMIT 1
 ) s ON true
 WHERE r.tenant_id=t AND r.company_id=c AND ds.id=d AND r.id>after_id
 ORDER BY r.id,l.resource_id
 LIMIT n;
END $$;
ALTER FUNCTION finance.runtime_reservations(uuid,uuid,uuid,uuid,uuid,integer) OWNER TO finance_reservation_reader;
REVOKE ALL ON FUNCTION finance.runtime_reservations(uuid,uuid,uuid,uuid,uuid,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_reservations(uuid,uuid,uuid,uuid,uuid,integer) TO finance_runtime;

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide','match.confirm'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED','MATCH_CONFIRMED'));
