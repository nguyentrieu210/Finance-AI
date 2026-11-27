-- E20 queued export request and fenced worker completion.
ALTER TABLE finance.package
 ADD COLUMN job_id uuid,
 ADD COLUMN requested_by uuid,
 ADD COLUMN dossier_version bigint CHECK(dossier_version BETWEEN 0 AND 9007199254740991),
 ADD COLUMN artifact_count integer CHECK(artifact_count BETWEEN 1 AND 256),
 ADD COLUMN artifact_bytes bigint CHECK(artifact_bytes BETWEEN 1 AND 26843545600),
 ADD CONSTRAINT package_job_fk FOREIGN KEY(tenant_id,company_id,job_id)
  REFERENCES finance.outbox(tenant_id,company_id,id),
 ADD CONSTRAINT package_requester_fk FOREIGN KEY(tenant_id,requested_by)
  REFERENCES finance.identity_subject(tenant_id,id),
 ADD CONSTRAINT package_runtime_binding CHECK(
  (job_id IS NULL AND requested_by IS NULL AND dossier_version IS NULL)
  OR (job_id IS NOT NULL AND requested_by IS NOT NULL AND dossier_version IS NOT NULL)),
 ADD CONSTRAINT package_artifact_receipt CHECK(
  (state='READY' AND manifest_hash IS NOT NULL)
  OR (state IN ('QUEUED','FAILED') AND manifest_hash IS NULL)
 );

CREATE FUNCTION finance.package_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'immutable package' USING ERRCODE='23514'; END IF;
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.snapshot_id,NEW.template_id,NEW.job_id,NEW.requested_by,NEW.dossier_version,NEW.created_at)
    IS DISTINCT FROM
    (OLD.tenant_id,OLD.company_id,OLD.id,OLD.snapshot_id,OLD.template_id,OLD.job_id,OLD.requested_by,OLD.dossier_version,OLD.created_at)
 THEN RAISE EXCEPTION 'immutable package binding' USING ERRCODE='23514'; END IF;
 IF OLD.state<>'QUEUED' OR NEW.state NOT IN ('READY','FAILED') THEN
  RAISE EXCEPTION 'invalid package transition' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER package_immutable BEFORE UPDATE OR DELETE ON finance.package
 FOR EACH ROW EXECUTE FUNCTION finance.package_guard();

CREATE POLICY package_requester_read ON finance.package FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.dossier_snapshot s JOIN finance.dossier d
 ON (d.tenant_id,d.company_id,d.id)=(s.tenant_id,s.company_id,s.dossier_id)
 WHERE (s.tenant_id,s.company_id,s.id)=(package.tenant_id,package.company_id,package.snapshot_id)
 AND d.owner_id=finance.scope_actor())
);
CREATE POLICY package_requester_insert ON finance.package FOR INSERT TO finance_runtime WITH CHECK(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND requested_by=finance.scope_actor() AND state='QUEUED' AND manifest_hash IS NULL
 AND artifact_count IS NULL AND artifact_bytes IS NULL
 AND EXISTS(SELECT 1 FROM finance.dossier_snapshot s JOIN finance.dossier d
 ON (d.tenant_id,d.company_id,d.id)=(s.tenant_id,s.company_id,s.dossier_id)
 WHERE (s.tenant_id,s.company_id,s.id)=(package.tenant_id,package.company_id,package.snapshot_id)
 AND d.owner_id=finance.scope_actor() AND d.status='READY' AND d.version=package.dossier_version
 AND s.template_version_id=package.template_id)
);
GRANT SELECT,INSERT ON finance.package TO finance_runtime;

CREATE ROLE finance_export_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_export_writer;
GRANT SELECT ON finance.package,finance.dossier_snapshot,finance.dossier,finance.job,finance.job_attempt,finance.worker_scope TO finance_export_writer;
GRANT UPDATE(state,manifest_hash,artifact_count,artifact_bytes) ON finance.package TO finance_export_writer;
GRANT UPDATE(state,lease_until,leased_by) ON finance.job TO finance_export_writer;
GRANT UPDATE(state,finished_at) ON finance.job_attempt TO finance_export_writer;
GRANT INSERT ON finance.inbox TO finance_export_writer;
GRANT EXECUTE ON FUNCTION finance.worker_scope_allowed(uuid,uuid) TO finance_export_writer;
CREATE POLICY package_export_writer ON finance.package TO finance_export_writer
 USING(finance.worker_scope_allowed(tenant_id,company_id))
 WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
CREATE POLICY job_export_writer ON finance.job TO finance_export_writer
 USING(finance.worker_scope_allowed(tenant_id,company_id))
 WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
CREATE POLICY attempt_export_writer ON finance.job_attempt TO finance_export_writer
 USING(finance.worker_scope_allowed(tenant_id,company_id))
 WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
CREATE POLICY inbox_export_writer ON finance.inbox FOR INSERT TO finance_export_writer
 WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id));
CREATE POLICY worker_scope_export_writer ON finance.worker_scope FOR SELECT TO finance_export_writer
 USING(login_name=session_user AND active);

CREATE FUNCTION finance.workflow_exported(
 t uuid,c uuid,d uuid,v bigint,a uuid,package_id uuid,cmd uuid,corr uuid
) RETURNS bigint
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; new_version bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR d IS NULL OR a IS NULL OR package_id IS NULL OR cmd IS NULL OR corr IS NULL
 OR v IS NULL OR v<0 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR a IS DISTINCT FROM finance.scope_actor()
 THEN RAISE EXCEPTION 'invalid export completion' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id<>a OR ds.status<>'READY' OR ds.version<>v THEN
  RAISE EXCEPTION 'stale export completion' USING ERRCODE='23514'; END IF;
 SELECT id INTO STRICT rev FROM finance.dossier_revision
 WHERE tenant_id=t AND company_id=c AND dossier_id=d AND number=ds.revision;
 IF NOT EXISTS(SELECT 1 FROM finance.package p JOIN finance.dossier_snapshot s
  ON (s.tenant_id,s.company_id,s.id)=(p.tenant_id,p.company_id,p.snapshot_id)
  WHERE p.tenant_id=t AND p.company_id=c AND p.id=package_id AND p.state='READY'
  AND s.dossier_id=d AND s.revision_id=rev)
 THEN RAISE EXCEPTION 'verified ready package required' USING ERRCODE='23514'; END IF;
 UPDATE finance.dossier SET status='EXPORTED',version=version+1,updated_at=GREATEST(updated_at,clock_timestamp())
 WHERE tenant_id=t AND company_id=c AND id=d RETURNING version INTO new_version;
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
 VALUES(t,c,gen_random_uuid(),a,cmd,corr,d,'EXPORT_COMPLETED');
 RETURN new_version;
END $$;
ALTER FUNCTION finance.workflow_exported(uuid,uuid,uuid,bigint,uuid,uuid,uuid,uuid)
 OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.workflow_exported(uuid,uuid,uuid,bigint,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.workflow_exported(uuid,uuid,uuid,bigint,uuid,uuid,uuid,uuid)
 TO finance_export_writer;

CREATE FUNCTION finance.complete_export_job(
 t uuid,c uuid,j uuid,f bigint,w uuid,manifest bytea,count integer,total_bytes bigint
) RETURNS bigint
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE p finance.package%ROWTYPE; s finance.dossier_snapshot%ROWTYPE; d finance.dossier%ROWTYPE; new_version bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR j IS NULL OR w IS NULL OR f IS NULL OR f<=0
 OR manifest IS NULL OR octet_length(manifest)<>32 OR count NOT BETWEEN 1 AND 256
 OR total_bytes NOT BETWEEN 1 AND 26843545600
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR NOT finance.worker_scope_allowed(t,c)
 THEN RAISE EXCEPTION 'invalid export worker completion' USING ERRCODE='23514'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.job WHERE tenant_id=t AND company_id=c AND id=j
  AND state='LEASED' AND fence=f AND leased_by=w AND lease_until>clock_timestamp() FOR UPDATE)
 THEN RAISE EXCEPTION 'stale export lease' USING ERRCODE='23514'; END IF;
 SELECT * INTO p FROM finance.package WHERE tenant_id=t AND company_id=c AND job_id=j AND state='QUEUED' FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'not an export job' USING ERRCODE='P0002'; END IF;
 SELECT * INTO STRICT s FROM finance.dossier_snapshot WHERE tenant_id=t AND company_id=c AND id=p.snapshot_id;
 SELECT * INTO STRICT d FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=s.dossier_id;
 IF p.requested_by IS NULL OR p.dossier_version IS NULL OR d.owner_id<>p.requested_by
 OR d.status<>'READY' OR d.version<>p.dossier_version
 THEN RAISE EXCEPTION 'stale export package' USING ERRCODE='23514'; END IF;
 UPDATE finance.package SET state='READY',manifest_hash=manifest,artifact_count=count,artifact_bytes=total_bytes
 WHERE tenant_id=t AND company_id=c AND id=p.id;
 PERFORM set_config('finance.actor_id',p.requested_by::text,true);
 new_version:=finance.workflow_exported(t,c,d.id,p.dossier_version,p.requested_by,p.id,j,j);
 UPDATE finance.job SET state='SUCCEEDED',lease_until=NULL,leased_by=NULL
 WHERE tenant_id=t AND company_id=c AND id=j AND fence=f AND leased_by=w AND state='LEASED';
 UPDATE finance.job_attempt SET state='SUCCEEDED',finished_at=clock_timestamp()
 WHERE tenant_id=t AND company_id=c AND job_id=j AND fence=f AND state='ACTIVE';
 INSERT INTO finance.inbox(tenant_id,company_id,event_id,fence) VALUES(t,c,j,f);
 RETURN new_version;
END $$;
ALTER FUNCTION finance.complete_export_job(uuid,uuid,uuid,bigint,uuid,bytea,integer,bigint)
 OWNER TO finance_export_writer;
REVOKE ALL ON FUNCTION finance.complete_export_job(uuid,uuid,uuid,bigint,uuid,bytea,integer,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.complete_export_job(uuid,uuid,uuid,bigint,uuid,bytea,integer,bigint)
 TO finance_job_worker;

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED'));
