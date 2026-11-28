-- Pin export input at READY; verified artifact bytes are bound to the worker fence.
ALTER TABLE finance.dossier_snapshot ADD COLUMN export_material jsonb CHECK(export_material IS NULL OR jsonb_typeof(export_material)='object');
ALTER TABLE finance.package ADD COLUMN artifact_fence bigint CHECK(artifact_fence>0),ADD COLUMN artifact_manifest jsonb;
ALTER TABLE finance.package ADD CHECK((artifact_fence IS NULL)=(artifact_manifest IS NULL)), ADD CHECK(artifact_manifest IS NULL OR (state='READY' AND jsonb_typeof(artifact_manifest)='array'));
GRANT UPDATE(artifact_fence,artifact_manifest) ON finance.package TO finance_export_writer;
GRANT SELECT ON finance.package TO finance_workflow_writer;
CREATE POLICY export_material_package_read ON finance.package FOR SELECT TO finance_workflow_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE OR REPLACE FUNCTION finance.workflow_ready(
 t uuid,c uuid,d uuid,v bigint,a uuid,check_id uuid,snap_id uuid,cmd uuid,corr uuid
) RETURNS uuid
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; cr finance.check_run%ROWTYPE;
 policy_row finance.policy_version%ROWTYPE; form_row finance.form_version%ROWTYPE;
 template_row finance.template_version%ROWTYPE; reservation_id uuid; manifest bytea; ack_manifest jsonb; render_material jsonb;
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
  AND revision_id=rev AND dossier_version=v AND actor_id=a;
 IF NOT FOUND OR cr.blocked OR cr.policy_id IS NULL
 OR EXISTS(SELECT 1 FROM finance.check_run newer WHERE newer.tenant_id=t AND newer.company_id=c
   AND newer.dossier_id=d AND newer.revision_id=rev AND newer.dossier_version=v
   AND (newer.created_at,newer.id)>(cr.created_at,cr.id))
 OR NOT EXISTS(SELECT 1 FROM finance.check_result x WHERE x.tenant_id=t AND x.company_id=c
   AND x.run_id=check_id)
 OR EXISTS(SELECT 1 FROM finance.check_result x WHERE x.tenant_id=t AND x.company_id=c
   AND x.run_id=check_id AND x.severity='BLOCK' AND x.outcome NOT IN ('PASS','NOT_APPLICABLE'))
 THEN RAISE EXCEPTION 'blocking checks' USING ERRCODE='23514'; END IF;
 IF EXISTS(SELECT 1 FROM finance.check_result x
  WHERE x.tenant_id=t AND x.company_id=c AND x.run_id=check_id
   AND x.severity='WARNING' AND x.outcome='FAIL' AND NOT EXISTS(
    SELECT 1 FROM finance.check_acknowledgment w
    WHERE (w.tenant_id,w.company_id,w.run_id,w.rule_key,w.actor_id)=(t,c,check_id,x.rule_key,a)))
 THEN RAISE EXCEPTION 'warning acknowledgment required' USING ERRCODE='23514'; END IF;
 SELECT COALESCE(jsonb_agg(jsonb_build_array(w.id,w.rule_key,w.actor_id,w.reason)
    ORDER BY w.rule_key,w.id),'[]'::jsonb) INTO ack_manifest
 FROM finance.check_acknowledgment w
 JOIN finance.check_result x ON (x.tenant_id,x.company_id,x.run_id,x.rule_key)=
  (w.tenant_id,w.company_id,w.run_id,w.rule_key)
 WHERE (w.tenant_id,w.company_id,w.run_id,w.actor_id)=(t,c,check_id,a)
  AND x.severity='WARNING' AND x.outcome='FAIL';
 PERFORM finance.prototype_policy_lock(t,c,ds.kind);
 SELECT * INTO policy_row FROM finance.policy_version
  WHERE tenant_id=t AND company_id=c AND id=cr.policy_id AND state='PUBLISHED'
  AND dossier_kind=ds.kind AND effective_from<=cr.business_day AND cr.business_day<effective_until;
 IF NOT FOUND THEN RAISE EXCEPTION 'effective policy unavailable' USING ERRCODE='23514'; END IF;
 IF EXISTS(SELECT 1 FROM finance.checklist_item ci WHERE ci.tenant_id=t AND ci.company_id=c
   AND ci.policy_id=cr.policy_id AND ci.mandatory AND NOT EXISTS(
    SELECT 1 FROM finance.upload_intent u JOIN finance.document_blob b
     ON (b.tenant_id,b.company_id,b.id)=(u.tenant_id,u.company_id,u.id)
    WHERE u.tenant_id=t AND u.company_id=c AND u.dossier_id=d AND u.revision_id=rev
     AND u.actor_id=a AND u.document_kind=ci.document_kind AND u.state='FINALIZED' AND b.scan_state='CLEAN'))
 THEN RAISE EXCEPTION 'mandatory document missing' USING ERRCODE='23514'; END IF;
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
 render_material:=jsonb_build_object(
 'schema','finance.export.material.v1','dossier',d,'revision',rev,'snapshot',snap_id,
 'template',template_row.id,'template_digest',encode(template_row.source_digest,'hex'),
 'fields',COALESCE((SELECT jsonb_agg(row.item ORDER BY row.path,row.document) FROM (
 SELECT f.field_path AS path,o.document_id AS document,jsonb_build_object('path',f.field_path,'document',o.document_id,'value',q.normalized_value,'evidence',q.evidence_ids) AS item
 FROM finance.document_ref dr JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(dr.tenant_id,dr.company_id,dr.id)
 JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id)=(dr.tenant_id,dr.company_id,dr.id)
 JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.document_id,o.revision_id,o.generation)=(dr.tenant_id,dr.company_id,dr.id,dr.revision_id,b.generation)
 JOIN finance.extraction_run x ON (x.tenant_id,x.company_id,x.ocr_run_id)=(o.tenant_id,o.company_id,o.id)
 JOIN finance.extracted_field f ON (f.tenant_id,f.company_id,f.run_id)=(x.tenant_id,x.company_id,x.id)
 JOIN LATERAL (SELECT q.disposition,q.normalized_value,q.evidence_ids FROM finance.field_decision q
 WHERE (q.tenant_id,q.company_id,q.run_id,q.field_path,q.dossier_id,q.revision_id)=(f.tenant_id,f.company_id,f.run_id,f.field_path,d,rev)
 AND q.dossier_version<=v ORDER BY q.dossier_version DESC,q.created_at DESC,q.id DESC LIMIT 1) q ON true
 WHERE (dr.tenant_id,dr.company_id,dr.dossier_id,dr.revision_id)=(t,c,d,rev)
 AND b.scan_state='CLEAN' AND u.state='FINALIZED' AND u.generation=b.generation AND o.source_digest=b.sha256_digest
 AND q.disposition IN ('CONFIRMED_CANDIDATE','CORRECTED') AND cardinality(q.evidence_ids)>0
 AND NOT EXISTS(SELECT 1 FROM finance.ocr_run newer WHERE (newer.tenant_id,newer.company_id,newer.document_id,newer.revision_id)=(o.tenant_id,o.company_id,o.document_id,o.revision_id)
 AND (newer.created_at,newer.id)>(o.created_at,o.id))
 LIMIT 5001) row),'[]'::jsonb),
 'documents',COALESCE((SELECT jsonb_agg(jsonb_build_object('id',b.id,'sha256',encode(b.sha256_digest,'hex'),'mime',b.mime,'bytes',b.byte_count,'kind',u.document_kind) ORDER BY b.id)
 FROM finance.upload_intent u JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(u.tenant_id,u.company_id,u.id)
 WHERE (u.tenant_id,u.company_id,u.dossier_id,u.revision_id)=(t,c,d,rev) AND u.state='FINALIZED' AND u.generation=b.generation AND b.scan_state='CLEAN'),'[]'::jsonb),
 'checks',COALESCE((SELECT jsonb_agg(jsonb_build_object('key',x.rule_key,'outcome',x.outcome,'severity',x.severity,'detail',x.detail_code,'version',x.rule_version,'evidence',x.evidence_ids) ORDER BY x.rule_key)
 FROM finance.check_result x WHERE (x.tenant_id,x.company_id,x.run_id)=(t,c,check_id)),'[]'::jsonb),
 'form',form_row.schema_json);
 IF jsonb_array_length(render_material->'fields')>5000 OR octet_length(render_material::text)>16777216 THEN RAISE EXCEPTION 'export material too large' USING ERRCODE='23514'; END IF;
 manifest:=sha256(convert_to(jsonb_build_array(
  d,rev,v,check_id,encode(cr.input_hash,'hex'),cr.policy_id,
  form_row.id,template_row.id,reservation_id,ack_manifest,render_material
 )::text,'UTF8'));
 INSERT INTO finance.dossier_snapshot(
  tenant_id,company_id,id,dossier_id,revision_id,policy_id,check_run_id,input_hash,
  canonical_manifest_hash,form_version_id,template_version_id,reservation_id,export_material
 ) VALUES(t,c,snap_id,d,rev,cr.policy_id,check_id,cr.input_hash,manifest,
  form_row.id,template_row.id,reservation_id,render_material);
 INSERT INTO finance.snapshot_member(tenant_id,company_id,snapshot_id,id,kind,resource_id,digest) VALUES
  (t,c,snap_id,gen_random_uuid(),'CHECK_RUN',check_id,cr.input_hash),
  (t,c,snap_id,gen_random_uuid(),'POLICY',policy_row.id,policy_row.source_digest),
  (t,c,snap_id,gen_random_uuid(),'FORM',form_row.id,sha256(convert_to(form_row.schema_json::text,'UTF8'))),
  (t,c,snap_id,gen_random_uuid(),'TEMPLATE',template_row.id,template_row.source_digest),
  (t,c,snap_id,gen_random_uuid(),'RESERVATION',reservation_id,
   sha256(convert_to(jsonb_build_array(reservation_id)::text,'UTF8')));
 INSERT INTO finance.snapshot_member(tenant_id,company_id,snapshot_id,id,kind,resource_id,digest)
 SELECT t,c,snap_id,gen_random_uuid(),'WARNING_ACK',w.id,
  sha256(convert_to(jsonb_build_array(w.id,w.rule_key,w.actor_id,w.reason)::text,'UTF8'))
 FROM finance.check_acknowledgment w JOIN finance.check_result x
  ON (x.tenant_id,x.company_id,x.run_id,x.rule_key)=(w.tenant_id,w.company_id,w.run_id,w.rule_key)
 WHERE (w.tenant_id,w.company_id,w.run_id,w.actor_id)=(t,c,check_id,a)
  AND x.severity='WARNING' AND x.outcome='FAIL';
 UPDATE finance.dossier SET status='READY',version=version+1,updated_at=GREATEST(updated_at,clock_timestamp())
  WHERE tenant_id=t AND company_id=c AND id=d;
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,d,'DOSSIER_READY');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,cmd,cmd,d,'DOSSIER_READY');
 RETURN snap_id;
END $$;

CREATE FUNCTION finance.runtime_export_material(t uuid,c uuid,j uuid,f bigint,w uuid)
RETURNS TABLE(material jsonb,document_id uuid,revision_id uuid,generation bigint,mime text,sha256 text,byte_count bigint,object_version text,content bytea,format text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT s.export_material,b.id,r.revision_id,b.generation,b.mime,encode(b.sha256_digest,'hex'),b.byte_count,b.object_version,b.synthetic_bytes,tv.format
 FROM finance.job q JOIN finance.package p ON (p.tenant_id,p.company_id,p.job_id)=(q.tenant_id,q.company_id,q.id)
 JOIN finance.dossier_snapshot s ON (s.tenant_id,s.company_id,s.id)=(p.tenant_id,p.company_id,p.snapshot_id)
 JOIN finance.dossier d ON (d.tenant_id,d.company_id,d.id)=(s.tenant_id,s.company_id,s.dossier_id)
 JOIN finance.template_version tv ON (tv.tenant_id,tv.company_id,tv.id)=(p.tenant_id,p.company_id,p.template_id)
 JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(tv.tenant_id,tv.company_id,tv.document_id)
 JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(b.tenant_id,b.company_id,b.id)
 WHERE (q.tenant_id,q.company_id,q.id,q.fence,q.leased_by)=(t,c,j,f,w)
 AND t=finance.scope_tenant() AND c=finance.scope_company() AND finance.worker_scope_allowed(t,c)
 AND q.state='LEASED' AND q.lease_until>statement_timestamp() AND p.state='QUEUED'
 AND d.status='READY' AND d.version=p.dossier_version AND d.owner_id=p.requested_by
 AND s.template_version_id=p.template_id AND s.export_material IS NOT NULL
 AND tv.state='PUBLISHED' AND b.scan_state='CLEAN' AND b.sha256_digest=tv.source_digest
$$;
ALTER FUNCTION finance.runtime_export_material(uuid,uuid,uuid,bigint,uuid) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.runtime_export_material(uuid,uuid,uuid,bigint,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_export_material(uuid,uuid,uuid,bigint,uuid) TO finance_job_worker;
CREATE FUNCTION finance.complete_export_job_verified(
 t uuid,c uuid,j uuid,f bigint,w uuid,manifest bytea,count integer,total_bytes bigint,artifacts jsonb
) RETURNS bigint
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE p finance.package%ROWTYPE; s finance.dossier_snapshot%ROWTYPE; d finance.dossier%ROWTYPE; new_version bigint; item jsonb; computed bytea:=convert_to('finance.package.manifest/v1','UTF8'); summed bigint:=0; names text[]:=ARRAY[]::text[]; n text; h text; size bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR j IS NULL OR w IS NULL OR f IS NULL OR f<=0
 OR manifest IS NULL OR octet_length(manifest)<>32 OR count NOT BETWEEN 1 AND 256
 OR total_bytes NOT BETWEEN 1 AND 26843545600
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR NOT finance.worker_scope_allowed(t,c)
 THEN RAISE EXCEPTION 'invalid export worker completion' USING ERRCODE='23514'; END IF;
 IF jsonb_typeof(artifacts) IS DISTINCT FROM 'array' OR jsonb_array_length(artifacts)<>count OR octet_length(artifacts::text)>262144
 THEN RAISE EXCEPTION 'invalid export manifest' USING ERRCODE='23514'; END IF;
 FOR item IN SELECT value FROM jsonb_array_elements(artifacts) ORDER BY (value->>'name') COLLATE "C" LOOP
  n:=item->>'name'; h:=item->>'sha256'; size:=(item->>'bytes')::bigint;
  IF n IS NULL OR n !~ '^[A-Za-z0-9_][A-Za-z0-9_.-]{0,127}$' OR n=ANY(names) OR h IS NULL OR h !~ '^[a-f0-9]{64}$' OR size IS NULL OR size NOT BETWEEN 1 AND 104857600
  THEN RAISE EXCEPTION 'invalid artifact' USING ERRCODE='23514'; END IF;
  names:=array_append(names,n); summed:=summed+size;
  computed:=computed||int8send(octet_length(n)::bigint)||convert_to(n,'UTF8')||decode(h,'hex')||int8send(size);
 END LOOP;
 IF NOT ('bundle.zip'=ANY(names)) OR summed<>total_bytes OR sha256(computed)<>manifest
 THEN RAISE EXCEPTION 'artifact manifest digest mismatch' USING ERRCODE='23514'; END IF;
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
 UPDATE finance.package SET state='READY',manifest_hash=manifest,artifact_count=count,artifact_bytes=total_bytes,artifact_fence=f,artifact_manifest=artifacts
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
ALTER FUNCTION finance.complete_export_job_verified(uuid,uuid,uuid,bigint,uuid,bytea,integer,bigint,jsonb)
 OWNER TO finance_export_writer;
REVOKE ALL ON FUNCTION finance.complete_export_job_verified(uuid,uuid,uuid,bigint,uuid,bytea,integer,bigint,jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.complete_export_job_verified(uuid,uuid,uuid,bigint,uuid,bytea,integer,bigint,jsonb)
 TO finance_job_worker;


CREATE FUNCTION finance.runtime_export_bundle(t uuid,c uuid,a uuid,d uuid,pkg uuid)
RETURNS TABLE(job_id uuid,artifact_fence bigint,sha256 text,byte_count bigint)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT p.job_id,p.artifact_fence,x->>'sha256',(x->>'bytes')::bigint
 FROM finance.package p CROSS JOIN LATERAL jsonb_array_elements(p.artifact_manifest) x
 WHERE (p.tenant_id,p.company_id,p.id)=(t,c,pkg) AND x->>'name'='bundle.zip'
 AND finance.runtime_export_manifest(t,c,a,d,pkg) IS NOT NULL
$$;
ALTER FUNCTION finance.runtime_export_bundle(uuid,uuid,uuid,uuid,uuid) OWNER TO finance_export_download;
REVOKE ALL ON FUNCTION finance.runtime_export_bundle(uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_export_bundle(uuid,uuid,uuid,uuid,uuid) TO finance_runtime;
