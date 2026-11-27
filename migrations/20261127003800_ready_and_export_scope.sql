-- E18 immutable check runs need no row lock. Serialize policy effectiveness
-- with the same company/kind advisory lock used by policy publication.
GRANT EXECUTE ON FUNCTION finance.prototype_policy_lock(uuid,uuid,text) TO finance_workflow_writer;
CREATE OR REPLACE FUNCTION finance.workflow_ready(
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
  AND revision_id=rev AND dossier_version=v;
 IF NOT FOUND OR cr.blocked OR cr.policy_id IS NULL
 OR EXISTS(SELECT 1 FROM finance.check_result x WHERE x.tenant_id=t AND x.company_id=c
   AND x.run_id=check_id AND x.severity='BLOCK' AND x.outcome NOT IN ('PASS','NOT_APPLICABLE'))
 THEN RAISE EXCEPTION 'blocking checks' USING ERRCODE='23514'; END IF;
 PERFORM finance.prototype_policy_lock(t,c,ds.kind);
 SELECT * INTO policy_row FROM finance.policy_version
  WHERE tenant_id=t AND company_id=c AND id=cr.policy_id AND state='PUBLISHED'
  AND dossier_kind=ds.kind AND effective_from<=cr.business_day AND cr.business_day<effective_until;
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
-- E20: only scoped job workers may read the sealed parents needed to finish export.
CREATE POLICY snapshot_export_writer_read ON finance.dossier_snapshot FOR SELECT TO finance_export_writer
 USING(finance.worker_scope_allowed(tenant_id,company_id));
CREATE POLICY dossier_export_writer_read ON finance.dossier FOR SELECT TO finance_export_writer
 USING(finance.worker_scope_allowed(tenant_id,company_id));
