-- E23 manual bridge and reconciliation. Live DO/ERP connectors remain disabled.
ALTER TABLE finance.handoff ADD COLUMN evidence_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[];
ALTER TABLE finance.handoff ADD CONSTRAINT handoff_reference_bounds CHECK(
 (external_ref IS NULL OR (octet_length(external_ref) BETWEEN 1 AND 256 AND btrim(external_ref)<>'')) AND
 cardinality(evidence_ids)<=64
);
ALTER TABLE finance.reconciliation_case
 ADD COLUMN channel text NOT NULL DEFAULT 'DO' CHECK(channel IN ('DO','ERP')),
 ADD COLUMN version bigint NOT NULL DEFAULT 0 CHECK(version BETWEEN 0 AND 9007199254740991),
 ADD COLUMN resolution_reason text,
 ADD COLUMN evidence_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[],
 ADD COLUMN resolution_evidence_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[];
ALTER TABLE finance.reconciliation_case ADD CONSTRAINT reconciliation_evidence_bounds CHECK(
 cardinality(evidence_ids)<=64 AND cardinality(resolution_evidence_ids)<=64 AND
 (resolution_reason IS NULL OR (octet_length(resolution_reason) BETWEEN 1 AND 4096 AND btrim(resolution_reason)<>''))
);

CREATE FUNCTION finance.reconciliation_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'immutable reconciliation case' USING ERRCODE='23514'; END IF;
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.handoff_id,NEW.channel,NEW.created_at,NEW.reason,NEW.evidence_ids)
    IS DISTINCT FROM
    (OLD.tenant_id,OLD.company_id,OLD.id,OLD.handoff_id,OLD.channel,OLD.created_at,OLD.reason,OLD.evidence_ids)
 THEN RAISE EXCEPTION 'immutable reconciliation binding' USING ERRCODE='23514'; END IF;
 IF OLD.state<>'OPEN' OR NEW.state<>'RESOLVED' OR NEW.version<>OLD.version+1
 OR NEW.resolved_at IS NULL OR NEW.resolution_reason IS NULL OR NEW.assigned_actor IS NULL
 THEN RAISE EXCEPTION 'invalid reconciliation resolution' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER reconciliation_case_guard BEFORE UPDATE OR DELETE ON finance.reconciliation_case
 FOR EACH ROW EXECUTE FUNCTION finance.reconciliation_guard();

-- The workflow writer is the only role allowed to perform the EXPORTED -> HANDED_OFF state change.
GRANT SELECT ON finance.package,finance.handoff,finance.evidence,finance.ocr_run,finance.document_ref TO finance_workflow_writer;
GRANT INSERT ON finance.handoff TO finance_workflow_writer;
CREATE POLICY workflow_package_read ON finance.package FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_handoff_read ON finance.handoff FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_handoff_insert ON finance.handoff FOR INSERT TO finance_workflow_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_evidence_read ON finance.evidence FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_ocr_read ON finance.ocr_run FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_document_ref_read ON finance.document_ref FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());

CREATE FUNCTION finance.integration_manual_handoff(
 t uuid,c uuid,d uuid,v bigint,a uuid,package uuid,handoff_id uuid,
 external text,evidence uuid[],cmd uuid,corr uuid
) RETURNS bigint
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; snap uuid; evidence_count bigint; new_version bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR d IS NULL OR a IS NULL OR package IS NULL OR handoff_id IS NULL
 OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0 OR external IS NULL
 OR octet_length(external) NOT BETWEEN 1 AND 256 OR btrim(external)=''
 OR evidence IS NULL OR cardinality(evidence)>64
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR a IS DISTINCT FROM finance.scope_actor()
 THEN RAISE EXCEPTION 'invalid manual handoff' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id<>a OR ds.status<>'EXPORTED' OR ds.version<>v
 OR NOT EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c
  AND m.actor_id=a AND m.role='REQUESTER' AND m.active
  AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'manual handoff authority or state' USING ERRCODE='23514'; END IF;
 SELECT id INTO STRICT rev FROM finance.dossier_revision
 WHERE tenant_id=t AND company_id=c AND dossier_id=d AND number=ds.revision;
 SELECT p.snapshot_id INTO snap FROM finance.package p
 JOIN finance.dossier_snapshot s ON (s.tenant_id,s.company_id,s.id)=(p.tenant_id,p.company_id,p.snapshot_id)
 WHERE p.tenant_id=t AND p.company_id=c AND p.id=package AND p.state='READY'
  AND s.dossier_id=d AND s.revision_id=rev;
 IF snap IS NULL THEN RAISE EXCEPTION 'ready package for current snapshot required' USING ERRCODE='23514'; END IF;
 IF cardinality(evidence)>0 THEN
  SELECT count(DISTINCT e.id) INTO evidence_count FROM finance.evidence e
  JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(e.tenant_id,e.company_id,e.ocr_run_id)
  JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(o.tenant_id,o.company_id,o.document_id)
  WHERE e.tenant_id=t AND e.company_id=c AND e.id=ANY(evidence)
   AND r.dossier_id=d AND r.revision_id=rev;
  IF evidence_count<>cardinality(evidence) THEN
   RAISE EXCEPTION 'handoff evidence binding' USING ERRCODE='23514'; END IF;
 END IF;
 INSERT INTO finance.handoff(tenant_id,company_id,id,snapshot_id,package_id,channel,state,external_ref,evidence_ids)
 VALUES(t,c,handoff_id,snap,package,'MANUAL','ACKNOWLEDGED',external,evidence);
 UPDATE finance.dossier SET status='HANDED_OFF',version=version+1,updated_at=GREATEST(updated_at,clock_timestamp())
 WHERE tenant_id=t AND company_id=c AND id=d RETURNING version INTO new_version;
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
 VALUES(t,c,cmd,a,cmd,corr,d,'MANUAL_HANDOFF_ACKNOWLEDGED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
 VALUES(t,c,cmd,cmd,d,'MANUAL_HANDOFF_ACKNOWLEDGED');
 RETURN new_version;
END $$;
ALTER FUNCTION finance.integration_manual_handoff(uuid,uuid,uuid,bigint,uuid,uuid,uuid,text,uuid[],uuid,uuid)
 OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.integration_manual_handoff(uuid,uuid,uuid,bigint,uuid,uuid,uuid,text,uuid[],uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.integration_manual_handoff(uuid,uuid,uuid,bigint,uuid,uuid,uuid,text,uuid[],uuid,uuid) TO finance_runtime;

CREATE POLICY handoff_requester_read ON finance.handoff FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.dossier_snapshot s JOIN finance.dossier d
 ON (d.tenant_id,d.company_id,d.id)=(s.tenant_id,s.company_id,s.dossier_id)
 WHERE (s.tenant_id,s.company_id,s.id)=(handoff.tenant_id,handoff.company_id,handoff.snapshot_id)
 AND d.owner_id=finance.scope_actor())
);
GRANT SELECT ON finance.handoff TO finance_runtime;

CREATE POLICY reconciliation_requester_read ON finance.reconciliation_case FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.handoff h JOIN finance.dossier_snapshot s
 ON (s.tenant_id,s.company_id,s.id)=(h.tenant_id,h.company_id,h.snapshot_id)
 JOIN finance.dossier d ON (d.tenant_id,d.company_id,d.id)=(s.tenant_id,s.company_id,s.dossier_id)
 WHERE (h.tenant_id,h.company_id,h.id)=(reconciliation_case.tenant_id,reconciliation_case.company_id,reconciliation_case.handoff_id)
 AND d.owner_id=finance.scope_actor())
);
CREATE POLICY reconciliation_approver_read ON finance.reconciliation_case FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=finance.scope_tenant()
 AND m.company_id=finance.scope_company() AND m.actor_id=finance.scope_actor()
 AND m.role='FINANCE_APPROVER' AND m.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
);
CREATE POLICY reconciliation_approver_update ON finance.reconciliation_case FOR UPDATE TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND state='OPEN'
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=finance.scope_tenant()
 AND m.company_id=finance.scope_company() AND m.actor_id=finance.scope_actor()
 AND m.role='FINANCE_APPROVER' AND m.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
) WITH CHECK(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND state='RESOLVED' AND assigned_actor=finance.scope_actor()
);
GRANT SELECT ON finance.reconciliation_case TO finance_runtime;
GRANT UPDATE(state,assigned_actor,resolved_at,resolution_reason,resolution_evidence_ids,version)
 ON finance.reconciliation_case TO finance_runtime;

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED'));
