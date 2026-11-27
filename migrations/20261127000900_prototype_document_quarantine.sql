-- E05 isolated synthetic fixture storage ONLY. Never store real documents here.
-- Bytea is a test vehicle to prove atomic metadata/content invariants, not the production S3 design.
ALTER TABLE finance.upload_intent DROP CONSTRAINT upload_intent_state_check,
 ADD CONSTRAINT upload_intent_state_check CHECK(state IN ('PENDING_UPLOAD','CANCELLED','FINALIZED')),
 ADD CONSTRAINT upload_intent_revision_identity UNIQUE(tenant_id,company_id,id,dossier_id,revision_id);
DROP INDEX finance.upload_quota_scope;
CREATE INDEX upload_quota_scope ON finance.upload_intent(tenant_id,company_id,dossier_id) WHERE state IN ('PENDING_UPLOAD','FINALIZED');
CREATE TABLE finance.document_blob (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 generation bigint NOT NULL,
 byte_count bigint NOT NULL CHECK(byte_count BETWEEN 1 AND 26214400),
 sha256_digest bytea NOT NULL CHECK(octet_length(sha256_digest)=32),
 mime text NOT NULL CHECK(mime IN ('application/pdf','image/jpeg','image/png')),
 synthetic_bytes bytea NOT NULL,
 storage_backend text NOT NULL DEFAULT 'SYNTHETIC_DB_ONLY' CHECK(storage_backend='SYNTHETIC_DB_ONLY'),
 scan_state text NOT NULL DEFAULT 'QUARANTINED' CHECK(scan_state='QUARANTINED'),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,id,generation) REFERENCES finance.upload_grant(tenant_id,company_id,intent_id,generation),
 CHECK(octet_length(synthetic_bytes)=byte_count AND sha256(synthetic_bytes)=sha256_digest)
);
CREATE TABLE finance.document_ref (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 dossier_id uuid NOT NULL, revision_id uuid NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,id) REFERENCES finance.document_blob(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,id,dossier_id,revision_id)
 REFERENCES finance.upload_intent(tenant_id,company_id,id,dossier_id,revision_id)
);
ALTER TABLE finance.document_blob ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.document_blob FORCE ROW LEVEL SECURITY;
ALTER TABLE finance.document_ref ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.document_ref FORCE ROW LEVEL SECURITY;
-- No policies or application grants. Only privileged disposable fixture owner.
CREATE FUNCTION finance.reject_synthetic_document_mutation() RETURNS trigger
LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 RAISE EXCEPTION 'synthetic document immutable' USING ERRCODE='23514';
END $$;
REVOKE ALL ON FUNCTION finance.reject_synthetic_document_mutation() FROM PUBLIC;
CREATE TRIGGER immutable_blob BEFORE UPDATE OR DELETE ON finance.document_blob
 FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE TRIGGER immutable_ref BEFORE UPDATE OR DELETE ON finance.document_ref
 FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
-- Preserve migration 00800; replace only the function in a forward migration.
CREATE OR REPLACE FUNCTION finance.prototype_reserve_upload(t uuid,c uuid,i uuid,d uuid,r uuid,a uuid,
 fname text,n bigint,h bytea,mime text) RETURNS uuid
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE used bigint;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' THEN
  RAISE EXCEPTION 'requires read committed' USING ERRCODE='25000';
 END IF;
 IF n IS NULL OR n<1 OR n>26214400 THEN RAISE EXCEPTION 'invalid size' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.upload.quota/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT COALESCE(sum(declared_bytes),0) INTO used FROM finance.upload_intent
  WHERE tenant_id=t AND company_id=c AND dossier_id=d AND state IN ('PENDING_UPLOAD','FINALIZED');
 IF used+n>104857600 THEN RAISE EXCEPTION 'quota exceeded' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.upload_intent(tenant_id,company_id,id,dossier_id,revision_id,actor_id,filename,declared_bytes,declared_sha256,declared_mime)
 VALUES(t,c,i,d,r,a,fname,n,h,mime);
 RETURN i;
END $$;
REVOKE ALL ON FUNCTION finance.prototype_reserve_upload(uuid,uuid,uuid,uuid,uuid,uuid,text,bigint,bytea,text) FROM PUBLIC;
-- Test owner supplies exact synthetic bytes. No client checksum attestation or storage locator is trusted.
-- Grant generation, declared metadata and revision binding checked under transaction locks.
CREATE FUNCTION finance.prototype_finalize_synthetic_upload(t uuid,c uuid,i uuid,r uuid,a uuid,g bigint,payload bytea)
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
REVOKE ALL ON FUNCTION finance.prototype_finalize_synthetic_upload(uuid,uuid,uuid,uuid,uuid,bigint,bytea) FROM PUBLIC;
-- No AV verdict, CLEAN transition, download, runtime access, delete API or real provider.
