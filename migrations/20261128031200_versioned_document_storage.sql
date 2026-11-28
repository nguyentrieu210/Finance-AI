-- E05 version-pinned external originals. Existing fixture rows remain unchanged.
ALTER TABLE finance.document_blob ALTER COLUMN synthetic_bytes DROP NOT NULL,
 DROP CONSTRAINT document_blob_storage_backend_check,
 ADD COLUMN staging_version text,
 ADD COLUMN object_version text,
 ADD CONSTRAINT document_storage_identity CHECK(
 (storage_backend='SYNTHETIC_DB_ONLY' AND synthetic_bytes IS NOT NULL AND staging_version IS NULL AND object_version IS NULL)
 OR (storage_backend='S3_VERSIONED' AND synthetic_bytes IS NULL
 AND staging_version IS NOT NULL AND staging_version ~ '^[!-~]{1,1024}$' AND staging_version<>'null'
 AND object_version IS NOT NULL AND object_version ~ '^[!-~]{1,1024}$' AND object_version<>'null')
 );
CREATE OR REPLACE FUNCTION finance.document_blob_scan_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'immutable document blob' USING ERRCODE='23514'; END IF;
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.generation,NEW.byte_count,NEW.sha256_digest,NEW.mime,NEW.synthetic_bytes,NEW.storage_backend,NEW.created_at,NEW.staging_version,NEW.object_version)
 IS DISTINCT FROM
 (OLD.tenant_id,OLD.company_id,OLD.id,OLD.generation,OLD.byte_count,OLD.sha256_digest,OLD.mime,OLD.synthetic_bytes,OLD.storage_backend,OLD.created_at,OLD.staging_version,OLD.object_version)
 THEN RAISE EXCEPTION 'immutable original identity' USING ERRCODE='23514'; END IF;
 IF OLD.scan_state<>'QUARANTINED' OR NEW.scan_state NOT IN ('CLEAN','REJECTED')
 THEN RAISE EXCEPTION 'invalid scan transition' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;

GRANT SELECT(object_version,storage_backend,generation) ON finance.document_blob TO finance_document_writer;
GRANT INSERT ON finance.document_blob,finance.document_ref TO finance_document_writer;
CREATE POLICY s3_document_writer_blob_insert ON finance.document_blob FOR INSERT TO finance_document_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
 AND storage_backend='S3_VERSIONED' AND scan_state='QUARANTINED');
CREATE POLICY s3_document_writer_ref_insert ON finance.document_ref FOR INSERT TO finance_document_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());

CREATE FUNCTION finance.runtime_storage_metadata(t uuid,c uuid,a uuid,d uuid,doc uuid,g bigint)
RETURNS TABLE(revision_id uuid,generation bigint,byte_count bigint,mime text,sha256 text,object_version text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT u.revision_id,u.generation,u.declared_bytes,u.declared_mime,encode(u.declared_sha256,'hex'),b.object_version
 FROM finance.upload_intent u
 JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(u.tenant_id,u.company_id,u.dossier_id)
 JOIN finance.dossier_revision r ON (r.tenant_id,r.company_id,r.dossier_id,r.id,r.number)=(ds.tenant_id,ds.company_id,ds.id,u.revision_id,ds.revision)
 LEFT JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id,b.generation)=(u.tenant_id,u.company_id,u.id,u.generation)
 WHERE (u.tenant_id,u.company_id,u.actor_id,u.dossier_id,u.id,u.generation)=(t,c,a,d,doc,g)
 AND (t,c,a)=(finance.scope_tenant(),finance.scope_company(),finance.scope_actor())
 AND ds.owner_id=a AND u.state IN ('PENDING_UPLOAD','FINALIZED')
 AND (b.id IS NULL OR b.storage_backend='S3_VERSIONED')
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
   AND m.role='REQUESTER' AND m.active AND m.valid_from<=statement_timestamp()
   AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
$$;
ALTER FUNCTION finance.runtime_storage_metadata(uuid,uuid,uuid,uuid,uuid,bigint) OWNER TO finance_document_writer;
REVOKE ALL ON FUNCTION finance.runtime_storage_metadata(uuid,uuid,uuid,uuid,uuid,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_storage_metadata(uuid,uuid,uuid,uuid,uuid,bigint) TO finance_runtime;

GRANT SELECT ON finance.outbox TO finance_document_writer;
CREATE POLICY s3_document_writer_outbox_read ON finance.outbox FOR SELECT TO finance_document_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE FUNCTION finance.runtime_attach_s3_original(t uuid,c uuid,a uuid,d uuid,doc uuid,g bigint,j uuid,stage text,original text)
RETURNS void LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE u finance.upload_intent%ROWTYPE;
BEGIN
 IF (t,c,a) IS DISTINCT FROM (finance.scope_tenant(),finance.scope_company(),finance.scope_actor())
 OR stage IS NULL OR original IS NULL OR stage !~ '^[!-~]{1,1024}$' OR original !~ '^[!-~]{1,1024}$'
 OR stage='null' OR original='null' OR g<1 OR j IS NULL
 THEN RAISE EXCEPTION 'invalid storage receipt' USING ERRCODE='23514'; END IF;
 PERFORM 1 FROM finance.dossier ds WHERE (ds.tenant_id,ds.company_id,ds.id,ds.owner_id)=(t,c,d,a) FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'dossier forbidden' USING ERRCODE='42501'; END IF;
 SELECT * INTO u FROM finance.upload_intent WHERE (tenant_id,company_id,actor_id,dossier_id,id,generation)=(t,c,a,d,doc,g) FOR UPDATE;
 IF NOT FOUND OR u.state<>'FINALIZED' OR NOT EXISTS(SELECT 1 FROM finance.runtime_storage_metadata(t,c,a,d,doc,g))
 OR NOT EXISTS(SELECT 1 FROM finance.outbox o JOIN finance.idempotency_record ir
 ON (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.result_id)=(o.tenant_id,o.company_id,a,'document.finalize',doc)
 WHERE (o.tenant_id,o.company_id,o.id,o.resource_id,o.event_type)=(t,c,j,doc,'DOCUMENT_FINALIZE_REQUESTED')
 AND ir.result_payload->>'id'=j::text)
 THEN RAISE EXCEPTION 'finalize receipt not current' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.document_blob(tenant_id,company_id,id,generation,byte_count,sha256_digest,mime,storage_backend,staging_version,object_version)
 VALUES(t,c,doc,g,u.declared_bytes,u.declared_sha256,u.declared_mime,'S3_VERSIONED',stage,original);
 INSERT INTO finance.document_ref(tenant_id,company_id,id,dossier_id,revision_id)
 VALUES(t,c,doc,d,u.revision_id);
END $$;
ALTER FUNCTION finance.runtime_attach_s3_original(uuid,uuid,uuid,uuid,uuid,bigint,uuid,text,text) OWNER TO finance_document_writer;
REVOKE ALL ON FUNCTION finance.runtime_attach_s3_original(uuid,uuid,uuid,uuid,uuid,bigint,uuid,text,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_attach_s3_original(uuid,uuid,uuid,uuid,uuid,bigint,uuid,text,text) TO finance_runtime;

GRANT SELECT(generation,byte_count,object_version) ON finance.document_blob TO finance_field_review_writer;
CREATE OR REPLACE FUNCTION finance.runtime_original_digest(t uuid,c uuid,a uuid,d uuid,i uuid)
RETURNS text LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT encode(b.sha256_digest,'hex') FROM finance.document_blob b
 JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(b.tenant_id,b.company_id,b.id)
 JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id)=(r.tenant_id,r.company_id,r.id)
 JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(r.tenant_id,r.company_id,r.dossier_id)
 JOIN finance.dossier_revision dr ON (dr.tenant_id,dr.company_id,dr.id,dr.dossier_id,dr.number)=(r.tenant_id,r.company_id,r.revision_id,d,ds.revision)
 WHERE (b.tenant_id,b.company_id,b.id)=(t,c,i) AND r.dossier_id=d
 AND b.scan_state='CLEAN' AND b.storage_backend IN ('SYNTHETIC_DB_ONLY','S3_VERSIONED') AND u.state='FINALIZED'
 AND finance.field_access_allowed(t,c,d,a)
$$;

CREATE FUNCTION finance.runtime_s3_download(t uuid,c uuid,a uuid,d uuid,i uuid,cmd uuid,corr uuid)
RETURNS TABLE(revision_id uuid,generation bigint,byte_count bigint,mime text,sha256 text,object_version text)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE digest text;
BEGIN
 IF cmd IS NULL OR corr IS NULL THEN RAISE EXCEPTION 'download context required' USING ERRCODE='22023'; END IF;
 digest:=finance.runtime_original_digest(t,c,a,d,i);
 IF digest IS NULL THEN RETURN; END IF;
 RETURN QUERY SELECT r.revision_id,b.generation,b.byte_count,b.mime,encode(b.sha256_digest,'hex'),b.object_version
 FROM finance.document_blob b JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(b.tenant_id,b.company_id,b.id)
 WHERE (b.tenant_id,b.company_id,b.id)=(t,c,i) AND r.dossier_id=d
 AND b.scan_state='CLEAN' AND b.storage_backend='S3_VERSIONED' AND encode(b.sha256_digest,'hex')=digest;
 IF FOUND THEN
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
 VALUES(t,c,cmd,a,cmd,corr,i,'DOCUMENT_DOWNLOADED');
 END IF;
END $$;
ALTER FUNCTION finance.runtime_s3_download(uuid,uuid,uuid,uuid,uuid,uuid,uuid) OWNER TO finance_field_review_writer;
REVOKE ALL ON FUNCTION finance.runtime_s3_download(uuid,uuid,uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_s3_download(uuid,uuid,uuid,uuid,uuid,uuid,uuid) TO finance_runtime;

-- Worker: both the scan disposition and the inbox/job receipt commit together.
GRANT UPDATE(scan_state,scan_engine,scan_engine_digest,scanned_at) ON finance.document_blob TO finance_workflow_writer;
CREATE POLICY s3_workflow_scan_update ON finance.document_blob FOR UPDATE TO finance_workflow_writer
 USING(finance.worker_scope_allowed(tenant_id,company_id) AND scan_state='QUARANTINED')
 WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id) AND scan_state IN ('CLEAN','REJECTED'));
CREATE FUNCTION finance.complete_s3_scan(t uuid,c uuid,j uuid,f bigint,w uuid,doc uuid,g bigint,v text,h text,verdict text,engine text)
RETURNS void LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $
DECLARE dossier uuid; revision uuid;
BEGIN
 IF NOT finance.worker_scope_allowed(t,c)
 OR verdict IS NULL OR verdict NOT IN ('CLEAN','REJECTED')
 OR h IS NULL OR h !~ '^[a-f0-9]{64}$' OR engine IS NULL OR engine !~ '^[a-f0-9]{64}$'
 THEN RAISE EXCEPTION 'scan authority or result' USING ERRCODE='42501'; END IF;
 PERFORM 1 FROM finance.job q JOIN finance.outbox o ON (o.tenant_id,o.company_id,o.id)=(q.tenant_id,q.company_id,q.id)
 WHERE (q.tenant_id,q.company_id,q.id,q.fence,q.leased_by)=(t,c,j,f,w)
 AND q.state='LEASED' AND q.lease_until>clock_timestamp() AND o.resource_id=doc AND o.event_type='DOCUMENT_FINALIZE_REQUESTED'
 FOR UPDATE OF q;
 IF NOT FOUND THEN RAISE EXCEPTION 'stale scan lease' USING ERRCODE='40001'; END IF;
 SELECT r.dossier_id,r.revision_id INTO dossier,revision FROM finance.document_ref r
 WHERE (r.tenant_id,r.company_id,r.id)=(t,c,doc);
 IF NOT FOUND THEN RAISE EXCEPTION 'scan source not found' USING ERRCODE='23514'; END IF;
 PERFORM 1 FROM finance.dossier ds JOIN finance.dossier_revision dr
 ON (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number)=(ds.tenant_id,ds.company_id,ds.id,ds.revision)
 WHERE (ds.tenant_id,ds.company_id,ds.id,dr.id)=(t,c,dossier,revision)
 AND ds.status IN ('DRAFT','PROCESSING','NEEDS_INPUT','REVIEWABLE') FOR UPDATE OF ds;
 IF NOT FOUND THEN RAISE EXCEPTION 'stale document revision' USING ERRCODE='23514'; END IF;
 UPDATE finance.document_blob b SET scan_state=verdict,scan_engine='CLAMAV',scan_engine_digest=decode(engine,'hex'),scanned_at=clock_timestamp()
 WHERE (b.tenant_id,b.company_id,b.id,b.generation,b.object_version)=(t,c,doc,g,v)
 AND b.sha256_digest=decode(h,'hex') AND b.storage_backend='S3_VERSIONED' AND b.scan_state='QUARANTINED';
 IF NOT FOUND THEN RAISE EXCEPTION 'source no longer quarantined' USING ERRCODE='23514'; END IF;
 UPDATE finance.dossier SET status=CASE WHEN status='REVIEWABLE' THEN 'DRAFT' ELSE status END,version=version+1,updated_at=GREATEST(updated_at,clock_timestamp())
 WHERE (tenant_id,company_id,id)=(t,c,dossier) AND version<9007199254740991;
 IF NOT FOUND THEN RAISE EXCEPTION 'dossier version exhausted' USING ERRCODE='23514'; END IF;
 UPDATE finance.job_attempt SET state='SUCCEEDED',finished_at=clock_timestamp()
 WHERE (tenant_id,company_id,job_id,fence,worker_id)=(t,c,j,f,w) AND state='ACTIVE';
 IF NOT FOUND THEN RAISE EXCEPTION 'stale scan attempt' USING ERRCODE='40001'; END IF;
 UPDATE finance.job SET state='SUCCEEDED',leased_by=NULL,lease_until=NULL
 WHERE (tenant_id,company_id,id,fence,leased_by)=(t,c,j,f,w) AND state='LEASED' AND lease_until>clock_timestamp();
 IF NOT FOUND THEN RAISE EXCEPTION 'stale scan lease' USING ERRCODE='40001'; END IF;
 INSERT INTO finance.inbox(tenant_id,company_id,event_id,fence) VALUES(t,c,j,f);
END $$;
ALTER FUNCTION finance.complete_s3_scan(uuid,uuid,uuid,bigint,uuid,uuid,bigint,text,text,text,text) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.complete_s3_scan(uuid,uuid,uuid,bigint,uuid,uuid,bigint,text,text,text,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.complete_s3_scan(uuid,uuid,uuid,bigint,uuid,uuid,bigint,text,text,text,text) TO finance_job_worker;
