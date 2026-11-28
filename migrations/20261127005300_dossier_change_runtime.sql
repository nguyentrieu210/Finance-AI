-- E18: owner revision/cancellation commands. No payment execution or automatic release.
ALTER TABLE finance.idempotency_record ADD COLUMN result_payload jsonb;
ALTER TABLE finance.idempotency_record ADD CONSTRAINT dossier_change_receipt_required CHECK(
 CASE WHEN operation_id IN ('dossier.revise','dossier.reopen','dossier.cancel')
 THEN result_payload IS NOT NULL AND jsonb_typeof(result_payload)='object'
 ELSE result_payload IS NULL END
);
GRANT INSERT ON finance.dossier_revision TO finance_workflow_writer;
CREATE POLICY workflow_revision_insert ON finance.dossier_revision FOR INSERT TO finance_workflow_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND owner_id=finance.scope_actor());
GRANT UPDATE(state) ON finance.upload_intent TO finance_workflow_writer;
CREATE POLICY workflow_cancel_upload ON finance.upload_intent FOR UPDATE TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor() AND state='PENDING_UPLOAD')
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor() AND state='CANCELLED');
CREATE OR REPLACE FUNCTION finance.prototype_dossier_version_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
DECLARE allowed boolean;
BEGIN
 IF TG_OP='INSERT' THEN
  IF NEW.version<>0 THEN RAISE EXCEPTION 'initial dossier version' USING ERRCODE='23514'; END IF;
  NEW.initial_updated_at:=NEW.updated_at;
  RETURN NEW;
 END IF;
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.owner_id,NEW.code,NEW.kind,NEW.initial_updated_at)
  IS DISTINCT FROM
  (OLD.tenant_id,OLD.company_id,OLD.id,OLD.owner_id,OLD.code,OLD.kind,OLD.initial_updated_at)
  OR NEW.version<>OLD.version+1 OR NEW.updated_at<OLD.updated_at THEN
   RAISE EXCEPTION 'invalid dossier mutation' USING ERRCODE='23514'; END IF;
 IF current_user<>'finance_workflow_writer' THEN
  IF (NEW.status,NEW.revision) IS DISTINCT FROM (OLD.status,OLD.revision) THEN
   RAISE EXCEPTION 'workflow mutation requires command boundary' USING ERRCODE='23514'; END IF;
  RETURN NEW;
 END IF;
 allowed:=(OLD.status,NEW.status) IN (
  ('DRAFT','PROCESSING'),('DRAFT','CANCELLED'),('NEEDS_INPUT','PROCESSING'),
  ('NEEDS_INPUT','CANCELLED'),('PROCESSING','REVIEWABLE'),('PROCESSING','NEEDS_INPUT'),
  ('REVIEWABLE','READY'),('REVIEWABLE','DRAFT'),('REVIEWABLE','CANCELLED'),
  ('READY','EXPORTED'),('READY','DRAFT'),('READY','CANCELLED'),
  ('EXPORTED','HANDED_OFF'),('EXPORTED','DRAFT'),('EXPORTED','CANCELLED'),
  ('HANDED_OFF','ACCOUNTING_REVIEW'),('ACCOUNTING_REVIEW','ACCEPTED'),
  ('ACCOUNTING_REVIEW','RETURNED'),('RETURNED','DRAFT'),('RETURNED','CANCELLED'));
 IF (OLD.status,NEW.status) IN (('DRAFT','DRAFT'),('NEEDS_INPUT','DRAFT')) THEN
  allowed:=NEW.revision=OLD.revision+1;
 END IF;
 IF NOT allowed OR NEW.revision<OLD.revision OR NEW.revision>OLD.revision+1
 OR (NEW.revision=OLD.revision+1 AND NEW.status<>'DRAFT') THEN
  RAISE EXCEPTION 'invalid dossier workflow transition' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;


CREATE FUNCTION finance.workflow_change_dossier(t uuid,c uuid,a uuid,d uuid,v bigint,op text,new_rev uuid,why text,evidence uuid[],cmd uuid,corr uuid)
RETURNS void LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; target text; action text;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR v IS NULL OR new_rev IS NULL
 OR cmd IS NULL OR corr IS NULL OR op IS NULL OR op NOT IN ('dossier.revise','dossier.reopen','dossier.cancel')
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR a IS DISTINCT FROM finance.scope_actor() OR why IS NULL OR octet_length(why) NOT BETWEEN 1 AND 2048
 OR btrim(why)='' OR why ~ '[[:cntrl:]]' OR evidence IS NULL OR cardinality(evidence)>64
 OR array_position(evidence,NULL::uuid) IS NOT NULL OR cardinality(evidence)<>(SELECT count(DISTINCT x) FROM unnest(evidence) x)
 THEN RAISE EXCEPTION 'invalid dossier change' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id<>a OR ds.version<>v OR v>=9007199254740991
 OR NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
   AND m.role='REQUESTER' AND m.active AND m.valid_from<=clock_timestamp()
   AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'dossier change authority or version' USING ERRCODE='23514'; END IF;
 SELECT id INTO STRICT rev FROM finance.dossier_revision WHERE tenant_id=t AND company_id=c AND dossier_id=d AND number=ds.revision;
 IF op='dossier.reopen' AND ds.status='RETURNED' THEN target:='DRAFT'; action:='DOSSIER_REOPENED';
 ELSIF op='dossier.revise' AND ds.status IN ('DRAFT','NEEDS_INPUT','REVIEWABLE') THEN target:='DRAFT'; action:='DOSSIER_REVISED';
 ELSIF op='dossier.cancel' AND ds.status IN ('DRAFT','NEEDS_INPUT','REVIEWABLE','RETURNED') THEN target:='CANCELLED'; action:='DOSSIER_CANCELLED';
 ELSE RAISE EXCEPTION 'dossier change state' USING ERRCODE='23514'; END IF;
 -- No automatic release of holds or reversal of consumed allocations.
 IF EXISTS(SELECT 1 FROM finance.reservation r WHERE r.tenant_id=t AND r.company_id=c AND r.revision_id=rev AND r.state<>'RELEASED')
 OR EXISTS(SELECT 1 FROM finance.dossier_snapshot s JOIN finance.package p
   ON (p.tenant_id,p.company_id,p.snapshot_id)=(s.tenant_id,s.company_id,s.id)
   WHERE s.tenant_id=t AND s.company_id=c AND s.dossier_id=d AND s.revision_id=rev AND p.state='QUEUED')
 OR EXISTS(SELECT 1 FROM finance.dossier_snapshot s JOIN finance.handoff h
   ON (h.tenant_id,h.company_id,h.snapshot_id)=(s.tenant_id,s.company_id,s.id)
   WHERE s.tenant_id=t AND s.company_id=c AND s.dossier_id=d AND s.revision_id=rev
   AND (ds.status<>'RETURNED' OR NOT EXISTS(SELECT 1 FROM finance.reconciliation_case rc
    WHERE (rc.tenant_id,rc.company_id,rc.handoff_id)=(h.tenant_id,h.company_id,h.id) AND rc.state='RESOLVED')
    OR EXISTS(SELECT 1 FROM finance.reconciliation_case rc
    WHERE (rc.tenant_id,rc.company_id,rc.handoff_id)=(h.tenant_id,h.company_id,h.id) AND rc.state<>'RESOLVED')))
 THEN RAISE EXCEPTION 'unresolved finance or external work' USING ERRCODE='23514'; END IF;
 IF cardinality(evidence)>0 AND (SELECT count(DISTINCT e.id) FROM finance.evidence e
  JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(e.tenant_id,e.company_id,e.ocr_run_id)
  JOIN finance.document_ref dr ON (dr.tenant_id,dr.company_id,dr.id)=(o.tenant_id,o.company_id,o.document_id)
  WHERE e.tenant_id=t AND e.company_id=c AND e.id=ANY(evidence) AND dr.dossier_id=d AND dr.revision_id=rev)<>cardinality(evidence)
 THEN RAISE EXCEPTION 'evidence outside current revision' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.upload.quota/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 UPDATE finance.upload_intent SET state='CANCELLED' WHERE tenant_id=t AND company_id=c AND dossier_id=d AND revision_id=rev
  AND actor_id=a AND state='PENDING_UPLOAD';
 IF target='DRAFT' THEN
  INSERT INTO finance.dossier_revision(tenant_id,company_id,dossier_id,id,owner_id,number) VALUES(t,c,d,new_rev,a,ds.revision+1);
 END IF;
 UPDATE finance.dossier SET status=target,revision=revision+CASE WHEN target='DRAFT' THEN 1 ELSE 0 END,
  version=version+1,updated_at=GREATEST(updated_at,clock_timestamp()) WHERE tenant_id=t AND company_id=c AND id=d;
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,d,action);
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,cmd,cmd,d,action);
END $$;
ALTER FUNCTION finance.workflow_change_dossier(uuid,uuid,uuid,uuid,bigint,text,uuid,text,uuid[],uuid,uuid) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.workflow_change_dossier(uuid,uuid,uuid,uuid,bigint,text,uuid,text,uuid[],uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.workflow_change_dossier(uuid,uuid,uuid,uuid,bigint,text,uuid,text,uuid[],uuid,uuid) TO finance_runtime;
ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','rule.revise','warning.ack','dossier.revise','dossier.reopen','dossier.cancel','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide','match.confirm','review.begin','review.decide','membership.create','membership.revoke',
 'risk.resolve','workflow.create','workflow.revise'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','WARNING_ACKNOWLEDGED','DOSSIER_REVISED','DOSSIER_REOPENED','DOSSIER_CANCELLED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','WARNING_ACKNOWLEDGED','DOSSIER_REVISED','DOSSIER_REOPENED','DOSSIER_CANCELLED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
