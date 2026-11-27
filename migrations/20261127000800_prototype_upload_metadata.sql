-- Isolated E05 schema candidate. Only migration/test owner may call these functions.
-- No dossier/revision authority until E18, no runtime/worker grant, no object-store or finalize.
CREATE TABLE finance.upload_intent (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 dossier_id uuid NOT NULL, revision_id uuid NOT NULL, actor_id uuid NOT NULL,
 filename text NOT NULL CHECK(octet_length(filename) BETWEEN 1 AND 255 AND filename=btrim(filename)
   AND filename NOT IN ('.','..') AND filename !~ '[[:cntrl:]/:]' AND position(chr(92) in filename)=0),
 declared_bytes bigint NOT NULL CHECK(declared_bytes BETWEEN 1 AND 26214400),
 declared_sha256 bytea NOT NULL CHECK(octet_length(declared_sha256)=32),
 declared_mime text NOT NULL CHECK(declared_mime IN ('application/pdf','image/jpeg','image/png')),
 state text NOT NULL DEFAULT 'PENDING_UPLOAD' CHECK(state IN ('PENDING_UPLOAD','CANCELLED')),
 generation bigint NOT NULL DEFAULT 0 CHECK(generation BETWEEN 0 AND 9007199254740991),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(id<>'00000000-0000-0000-0000-000000000000' AND dossier_id<>'00000000-0000-0000-0000-000000000000' AND revision_id<>'00000000-0000-0000-0000-000000000000')
);
CREATE INDEX upload_quota_scope ON finance.upload_intent(tenant_id,company_id,dossier_id) WHERE state='PENDING_UPLOAD';
CREATE TABLE finance.upload_grant (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, intent_id uuid NOT NULL,
 generation bigint NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
 staging_key text NOT NULL UNIQUE,
 issued_at timestamptz NOT NULL, expires_at timestamptz NOT NULL,
 PRIMARY KEY(tenant_id,company_id,intent_id,generation),
 FOREIGN KEY(tenant_id,company_id,intent_id) REFERENCES finance.upload_intent(tenant_id,company_id,id),
 CHECK(expires_at>issued_at AND expires_at<=issued_at+interval '300 seconds')
);
ALTER TABLE finance.upload_intent ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.upload_intent FORCE ROW LEVEL SECURITY;
ALTER TABLE finance.upload_grant ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.upload_grant FORCE ROW LEVEL SECURITY;
-- No policies or application grants: default deny. These functions are invoker-only and
-- only the privileged disposable fixture owner may execute; NOT trusted application entry points.
CREATE FUNCTION finance.prototype_reserve_upload(t uuid,c uuid,i uuid,d uuid,r uuid,a uuid,
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
  WHERE tenant_id=t AND company_id=c AND dossier_id=d AND state='PENDING_UPLOAD';
 IF used+n>104857600 THEN RAISE EXCEPTION 'quota exceeded' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.upload_intent(tenant_id,company_id,id,dossier_id,revision_id,actor_id,filename,declared_bytes,declared_sha256,declared_mime)
 VALUES(t,c,i,d,r,a,fname,n,h,mime);
 RETURN i;
END $$;
CREATE FUNCTION finance.prototype_issue_upload_grant(t uuid,c uuid,i uuid,r uuid,a uuid,ttl integer)
RETURNS bigint LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE row_intent finance.upload_intent%ROWTYPE; g bigint; issued timestamptz;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' OR ttl IS NULL OR ttl<1 OR ttl>300 THEN
  RAISE EXCEPTION 'invalid grant request' USING ERRCODE='23514';
 END IF;
 SELECT * INTO row_intent FROM finance.upload_intent
  WHERE tenant_id=t AND company_id=c AND id=i AND revision_id=r AND actor_id=a FOR UPDATE;
 IF NOT FOUND OR row_intent.state<>'PENDING_UPLOAD' OR row_intent.generation>=9007199254740991 THEN
  RAISE EXCEPTION 'inactive intent' USING ERRCODE='23514';
 END IF;
 g:=row_intent.generation+1; issued:=clock_timestamp();
 INSERT INTO finance.upload_grant VALUES(t,c,i,g,
  'staging/'||t::text||'/'||c::text||'/'||r::text||'/'||i::text||'/'||g::text,
  issued,issued+make_interval(secs=>ttl));
 UPDATE finance.upload_intent SET generation=g WHERE tenant_id=t AND company_id=c AND id=i;
 RETURN g;
END $$;
CREATE FUNCTION finance.prototype_cancel_upload(t uuid,c uuid,i uuid,r uuid,a uuid)
RETURNS void LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE d uuid;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' THEN RAISE EXCEPTION 'requires read committed' USING ERRCODE='25000'; END IF;
 SELECT dossier_id INTO d FROM finance.upload_intent WHERE tenant_id=t AND company_id=c AND id=i AND revision_id=r AND actor_id=a;
 IF NOT FOUND THEN RAISE EXCEPTION 'unknown intent' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.upload.quota/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 UPDATE finance.upload_intent SET state='CANCELLED' WHERE tenant_id=t AND company_id=c AND id=i AND state='PENDING_UPLOAD';
 IF NOT FOUND THEN RAISE EXCEPTION 'inactive intent' USING ERRCODE='23514'; END IF;
END $$;
CREATE FUNCTION finance.prototype_upload_grant_current(t uuid,c uuid,i uuid,r uuid,a uuid,g bigint)
RETURNS boolean LANGUAGE sql VOLATILE SET search_path=pg_catalog AS $$
 SELECT EXISTS(SELECT 1 FROM finance.upload_intent u JOIN finance.upload_grant v
 ON (u.tenant_id,u.company_id,u.id,u.generation)=(v.tenant_id,v.company_id,v.intent_id,v.generation)
 WHERE u.tenant_id=t AND u.company_id=c AND u.id=i AND u.revision_id=r AND u.actor_id=a
 AND u.state='PENDING_UPLOAD' AND u.generation=g AND v.expires_at>clock_timestamp())
$$;
REVOKE ALL ON FUNCTION finance.prototype_reserve_upload(uuid,uuid,uuid,uuid,uuid,uuid,text,bigint,bytea,text) FROM PUBLIC;
REVOKE ALL ON FUNCTION finance.prototype_issue_upload_grant(uuid,uuid,uuid,uuid,uuid,integer) FROM PUBLIC;
REVOKE ALL ON FUNCTION finance.prototype_cancel_upload(uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION finance.prototype_upload_grant_current(uuid,uuid,uuid,uuid,uuid,bigint) FROM PUBLIC;
-- No finalization, blob reference, scan result, runtime privileges, or deletion.
