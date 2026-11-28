-- E05 Office container admission plus fenced scanner verdict persistence.
ALTER TABLE finance.upload_intent DROP CONSTRAINT upload_intent_declared_mime_check,
 ADD CONSTRAINT upload_intent_declared_mime_check CHECK(declared_mime IN (
 'application/pdf','image/jpeg','image/png',
 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'));
ALTER TABLE finance.document_blob DROP CONSTRAINT document_blob_mime_check,
 ADD CONSTRAINT document_blob_mime_check CHECK(mime IN (
 'application/pdf','image/jpeg','image/png',
 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'));
ALTER TABLE finance.document_blob DROP CONSTRAINT document_blob_scan_state_check,
 ADD CONSTRAINT document_blob_scan_state_check CHECK(scan_state IN ('QUARANTINED','CLEAN','REJECTED')),
 ADD COLUMN scan_engine text,
 ADD COLUMN scan_engine_digest bytea,
 ADD COLUMN scanned_at timestamptz,
 ADD CONSTRAINT document_blob_scan_metadata CHECK(
  (scan_state='QUARANTINED' AND scan_engine IS NULL AND scan_engine_digest IS NULL AND scanned_at IS NULL)
  OR (scan_state IN ('CLEAN','REJECTED') AND scan_engine='CLAMAV'
      AND octet_length(scan_engine_digest)=32 AND scanned_at IS NOT NULL));

DROP TRIGGER immutable_blob ON finance.document_blob;
CREATE FUNCTION finance.document_blob_scan_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF TG_OP='DELETE' THEN RAISE EXCEPTION 'immutable document blob' USING ERRCODE='23514'; END IF;
 IF (NEW.tenant_id,NEW.company_id,NEW.id,NEW.generation,NEW.byte_count,NEW.sha256_digest,NEW.mime,NEW.synthetic_bytes,NEW.storage_backend,NEW.created_at)
    IS DISTINCT FROM
    (OLD.tenant_id,OLD.company_id,OLD.id,OLD.generation,OLD.byte_count,OLD.sha256_digest,OLD.mime,OLD.synthetic_bytes,OLD.storage_backend,OLD.created_at)
 THEN RAISE EXCEPTION 'immutable document bytes' USING ERRCODE='23514'; END IF;
 IF OLD.scan_state<>'QUARANTINED' OR NEW.scan_state NOT IN ('CLEAN','REJECTED')
 THEN RAISE EXCEPTION 'invalid scan transition' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER immutable_blob BEFORE UPDATE OR DELETE ON finance.document_blob
 FOR EACH ROW EXECUTE FUNCTION finance.document_blob_scan_guard();

CREATE POLICY document_blob_worker_read ON finance.document_blob FOR SELECT TO finance_job_worker
 USING(finance.worker_scope_allowed(tenant_id,company_id));
CREATE POLICY document_blob_worker_scan ON finance.document_blob FOR UPDATE TO finance_job_worker
 USING(finance.worker_scope_allowed(tenant_id,company_id) AND scan_state='QUARANTINED')
 WITH CHECK(finance.worker_scope_allowed(tenant_id,company_id) AND scan_state IN ('CLEAN','REJECTED'));
CREATE POLICY document_ref_worker_read ON finance.document_ref FOR SELECT TO finance_job_worker
 USING(finance.worker_scope_allowed(tenant_id,company_id));
GRANT SELECT ON finance.document_blob,finance.document_ref TO finance_job_worker;
GRANT UPDATE(scan_state,scan_engine,scan_engine_digest,scanned_at) ON finance.document_blob TO finance_job_worker;

CREATE OR REPLACE FUNCTION finance.prototype_finalize_synthetic_upload(t uuid,c uuid,i uuid,r uuid,a uuid,g bigint,payload bytea)
RETURNS uuid LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE d uuid; u finance.upload_intent%ROWTYPE; digest bytea; signature_ok boolean;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' THEN
  RAISE EXCEPTION 'requires read committed' USING ERRCODE='25000';
 END IF;
 IF payload IS NULL OR octet_length(payload)<1 OR octet_length(payload)>26214400 OR g IS NULL OR g<1 THEN
  RAISE EXCEPTION 'invalid payload or generation' USING ERRCODE='23514';
 END IF;
 SELECT dossier_id INTO d FROM finance.upload_intent WHERE tenant_id=t AND company_id=c AND id=i AND revision_id=r AND actor_id=a;
 IF NOT FOUND THEN RAISE EXCEPTION 'unknown intent' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.upload.quota/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO u FROM finance.upload_intent
 WHERE tenant_id=t AND company_id=c AND id=i AND revision_id=r AND actor_id=a FOR UPDATE;
 IF NOT FOUND OR u.generation<>g OR u.state='CANCELLED' THEN
  RAISE EXCEPTION 'inactive generation' USING ERRCODE='23514';
 END IF;
 digest:=sha256(payload);
 IF octet_length(payload)<>u.declared_bytes OR digest<>u.declared_sha256 THEN
  RAISE EXCEPTION 'size or digest mismatch' USING ERRCODE='23514';
 END IF;
 signature_ok:=CASE u.declared_mime
  WHEN 'application/pdf' THEN substring(payload FROM 1 FOR 5)=decode('255044462d','hex')
  WHEN 'image/jpeg' THEN substring(payload FROM 1 FOR 3)=decode('ffd8ff','hex')
  WHEN 'image/png' THEN substring(payload FROM 1 FOR 8)=decode('89504e470d0a1a0a','hex')
  WHEN 'application/vnd.openxmlformats-officedocument.wordprocessingml.document' THEN substring(payload FROM 1 FOR 4)=decode('504b0304','hex')
  WHEN 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' THEN substring(payload FROM 1 FOR 4)=decode('504b0304','hex')
  ELSE false END;
 IF NOT signature_ok THEN RAISE EXCEPTION 'signature mismatch' USING ERRCODE='23514'; END IF;
 IF u.state='FINALIZED' THEN
  IF EXISTS(SELECT 1 FROM finance.document_blob b JOIN finance.document_ref f
    ON (f.tenant_id,f.company_id,f.id)=(b.tenant_id,b.company_id,b.id)
    WHERE b.tenant_id=t AND b.company_id=c AND b.id=i AND b.generation=g AND b.synthetic_bytes=payload
    AND f.dossier_id=d AND f.revision_id=r) THEN RETURN i; END IF;
  RAISE EXCEPTION 'finalized receipt mismatch' USING ERRCODE='23514';
 END IF;
 IF NOT finance.prototype_upload_grant_current(t,c,i,r,a,g) THEN
  RAISE EXCEPTION 'expired or inactive grant' USING ERRCODE='23514';
 END IF;
 INSERT INTO finance.document_blob(tenant_id,company_id,id,generation,byte_count,sha256_digest,mime,synthetic_bytes)
 VALUES(t,c,i,g,u.declared_bytes,digest,u.declared_mime,payload);
 INSERT INTO finance.document_ref(tenant_id,company_id,id,dossier_id,revision_id)
 VALUES(t,c,i,d,r);
 UPDATE finance.upload_intent SET state='FINALIZED' WHERE tenant_id=t AND company_id=c AND id=i;
 RETURN i;
END $$;
