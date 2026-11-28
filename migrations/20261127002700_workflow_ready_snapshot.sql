-- E18 persisted READY transition and immutable sealed snapshot.
ALTER TABLE finance.dossier DROP CONSTRAINT IF EXISTS dossier_status_check,
 DROP CONSTRAINT IF EXISTS dossier_revision_check,
 ADD CONSTRAINT dossier_status_check CHECK(status IN (
  'DRAFT','PROCESSING','NEEDS_INPUT','REVIEWABLE','READY','EXPORTED',
  'HANDED_OFF','ACCOUNTING_REVIEW','ACCEPTED','RETURNED','CANCELLED')),
 ADD CONSTRAINT dossier_revision_check CHECK(revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE finance.dossier_revision DROP CONSTRAINT IF EXISTS dossier_revision_number_check,
 ADD CONSTRAINT dossier_revision_number_check CHECK(number BETWEEN 0 AND 9007199254740991);
ALTER TABLE finance.check_run DROP CONSTRAINT IF EXISTS check_run_blocked_check;

ALTER TABLE finance.dossier_snapshot ADD COLUMN form_version_id uuid,
 ADD COLUMN template_version_id uuid,
 ADD COLUMN reservation_id uuid,
 ADD CONSTRAINT dossier_snapshot_form_fk FOREIGN KEY(tenant_id,company_id,form_version_id)
  REFERENCES finance.form_version(tenant_id,company_id,id),
 ADD CONSTRAINT dossier_snapshot_template_fk FOREIGN KEY(tenant_id,company_id,template_version_id)
  REFERENCES finance.template_version(tenant_id,company_id,id),
 ADD CONSTRAINT dossier_snapshot_reservation_fk FOREIGN KEY(tenant_id,company_id,reservation_id)
  REFERENCES finance.reservation(tenant_id,company_id,id);

CREATE ROLE finance_workflow_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_workflow_writer;
GRANT SELECT ON finance.dossier,finance.dossier_revision,finance.check_run,finance.check_result,
 finance.policy_version,finance.form_version,finance.template_version,finance.reservation,
 finance.upload_intent,finance.document_blob,finance.dossier_snapshot,finance.snapshot_member TO finance_workflow_writer;
GRANT UPDATE(status,version,revision,updated_at) ON finance.dossier TO finance_workflow_writer;
GRANT INSERT ON finance.dossier_snapshot,finance.snapshot_member,finance.audit_event,finance.outbox TO finance_workflow_writer;

CREATE POLICY workflow_dossier_read ON finance.dossier FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_dossier_update ON finance.dossier FOR UPDATE TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_revision_read ON finance.dossier_revision FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_check_run_read ON finance.check_run FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_check_result_read ON finance.check_result FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_policy_read ON finance.policy_version FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_form_read ON finance.form_version FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_template_read ON finance.template_version FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_reservation_read ON finance.reservation FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_upload_read ON finance.upload_intent FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_blob_read ON finance.document_blob FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_snapshot_read ON finance.dossier_snapshot FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_snapshot_insert ON finance.dossier_snapshot FOR INSERT TO finance_workflow_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_member_read ON finance.snapshot_member FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_member_insert ON finance.snapshot_member FOR INSERT TO finance_workflow_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY workflow_audit_insert ON finance.audit_event FOR INSERT TO finance_workflow_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
  AND actor_id=finance.scope_actor());
CREATE POLICY workflow_outbox_insert ON finance.outbox FOR INSERT TO finance_workflow_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());

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
 IF NOT allowed OR NEW.revision<OLD.revision OR NEW.revision>OLD.revision+1
 OR (NEW.revision=OLD.revision+1 AND NEW.status<>'DRAFT') THEN
  RAISE EXCEPTION 'invalid dossier workflow transition' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;

CREATE FUNCTION finance.workflow_ready(
 t uuid,c uuid,d uuid,v bigint,a uuid,check_id uuid,snap_id uuid,cmd uuid,corr uuid
) RETURNS uuid
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; cr finance.check_run%ROWTYPE;
 policy_row finance.policy_version%ROWTYPE; form_row finance.form_version%ROWTYPE;
 template_row finance.template_version%ROWTYPE; reservation_id uuid; manifest bytea;
BEGIN
 IF t IS NULL OR c IS NULL OR d IS NULL OR a IS NULL OR check_id IS NULL OR snap_id IS NULL
 OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR a IS DISTINCT FROM finance.scope_actor()
 THEN RAISE EXCEPTION 'invalid ready command' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id<>a OR ds.status<>'REVIEWABLE' OR ds.version<>v
 OR NOT EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c
  AND m.actor_id=a AND m.role='REQUESTER' AND m.active
  AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'ready authority or state' USING ERRCODE='23514'; END IF;
 SELECT id INTO STRICT rev FROM finance.dossier_revision
  WHERE tenant_id=t AND company_id=c AND dossier_id=d AND number=ds.revision;
 SELECT * INTO cr FROM finance.check_run
  WHERE tenant_id=t AND company_id=c AND id=check_id AND dossier_id=d
  AND revision_id=rev AND dossier_version=v FOR SHARE;
 IF NOT FOUND OR cr.blocked OR cr.policy_id IS NULL
 OR EXISTS(SELECT 1 FROM finance.check_result x WHERE x.tenant_id=t AND x.company_id=c
   AND x.run_id=check_id AND x.severity='BLOCK' AND x.outcome NOT IN ('PASS','NOT_APPLICABLE'))
 THEN RAISE EXCEPTION 'blocking checks' USING ERRCODE='23514'; END IF;
 SELECT * INTO policy_row FROM finance.policy_version
  WHERE tenant_id=t AND company_id=c AND id=cr.policy_id AND state='PUBLISHED'
  AND effective_from<=cr.business_day AND cr.business_day<effective_until FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'effective policy unavailable' USING ERRCODE='23514'; END IF;
 SELECT * INTO form_row FROM finance.form_version WHERE tenant_id=t AND company_id=c
  AND policy_id=cr.policy_id AND state='PUBLISHED' ORDER BY version DESC,id DESC LIMIT 1;
 IF NOT FOUND THEN RAISE EXCEPTION 'published form unavailable' USING ERRCODE='23514'; END IF;
 SELECT * INTO template_row FROM finance.template_version WHERE tenant_id=t AND company_id=c
  AND policy_id=cr.policy_id AND state='PUBLISHED' ORDER BY version DESC,id DESC LIMIT 1;
 IF NOT FOUND THEN RAISE EXCEPTION 'published template unavailable' USING ERRCODE='23514'; END IF;
 SELECT id INTO reservation_id FROM finance.reservation WHERE tenant_id=t AND company_id=c
  AND revision_id=rev AND state='RESERVED' AND remaining>0 ORDER BY created_at DESC,id DESC LIMIT 1;
 IF reservation_id IS NULL THEN RAISE EXCEPTION 'active reservation required' USING ERRCODE='23514'; END IF;
 IF EXISTS(SELECT 1 FROM finance.upload_intent u WHERE u.tenant_id=t AND u.company_id=c
   AND u.dossier_id=d AND u.revision_id=rev AND u.state<>'CANCELLED'
   AND (u.state<>'FINALIZED' OR NOT EXISTS(SELECT 1 FROM finance.document_blob b
     WHERE b.tenant_id=u.tenant_id AND b.company_id=u.company_id AND b.id=u.id
     AND b.scan_state='CLEAN')))
 THEN RAISE EXCEPTION 'unclean document' USING ERRCODE='23514'; END IF;
 manifest:=sha256(convert_to(jsonb_build_array(
  d,rev,v,check_id,encode(cr.input_hash,'hex'),cr.policy_id,
  form_row.id,template_row.id,reservation_id
 )::text,'UTF8'));
 INSERT INTO finance.dossier_snapshot(
  tenant_id,company_id,id,dossier_id,revision_id,policy_id,check_run_id,input_hash,
  canonical_manifest_hash,form_version_id,template_version_id,reservation_id
 ) VALUES(t,c,snap_id,d,rev,cr.policy_id,check_id,cr.input_hash,manifest,
  form_row.id,template_row.id,reservation_id);
 INSERT INTO finance.snapshot_member(tenant_id,company_id,snapshot_id,id,kind,resource_id,digest) VALUES
  (t,c,snap_id,gen_random_uuid(),'CHECK_RUN',check_id,cr.input_hash),
  (t,c,snap_id,gen_random_uuid(),'POLICY',policy_row.id,policy_row.source_digest),
  (t,c,snap_id,gen_random_uuid(),'FORM',form_row.id,sha256(convert_to(form_row.schema_json::text,'UTF8'))),
  (t,c,snap_id,gen_random_uuid(),'TEMPLATE',template_row.id,template_row.source_digest),
  (t,c,snap_id,gen_random_uuid(),'RESERVATION',reservation_id,
   sha256(convert_to(jsonb_build_array(reservation_id)::text,'UTF8')));
 UPDATE finance.dossier SET status='READY',version=version+1,updated_at=GREATEST(updated_at,clock_timestamp())
  WHERE tenant_id=t AND company_id=c AND id=d;
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,d,'DOSSIER_READY');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,cmd,cmd,d,'DOSSIER_READY');
 RETURN snap_id;
END $$;
ALTER FUNCTION finance.workflow_ready(uuid,uuid,uuid,bigint,uuid,uuid,uuid,uuid,uuid)
 OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.workflow_ready(uuid,uuid,uuid,bigint,uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.workflow_ready(uuid,uuid,uuid,bigint,uuid,uuid,uuid,uuid,uuid)
 TO finance_runtime;

CREATE POLICY snapshot_owner_read ON finance.dossier_snapshot FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.dossier d WHERE
  (d.tenant_id,d.company_id,d.id)=(dossier_snapshot.tenant_id,dossier_snapshot.company_id,dossier_snapshot.dossier_id)
  AND d.owner_id=finance.scope_actor()));
CREATE POLICY snapshot_member_owner_read ON finance.snapshot_member FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.dossier_snapshot s JOIN finance.dossier d
  ON (d.tenant_id,d.company_id,d.id)=(s.tenant_id,s.company_id,s.dossier_id)
  WHERE (s.tenant_id,s.company_id,s.id)=(snapshot_member.tenant_id,snapshot_member.company_id,snapshot_member.snapshot_id)
  AND d.owner_id=finance.scope_actor()));
GRANT SELECT ON finance.dossier_snapshot,finance.snapshot_member TO finance_runtime;

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY'));
