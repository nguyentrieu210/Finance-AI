-- E05: authenticated short-lived access to immutable CLEAN synthetic originals.
-- No production storage credentials or public object-store links.
GRANT SELECT(tenant_id,company_id,id,scan_state,sha256_digest,mime,synthetic_bytes,storage_backend) ON finance.document_blob TO finance_field_review_writer;
GRANT SELECT ON finance.upload_intent TO finance_field_review_writer;
CREATE POLICY field_writer_download_blob ON finance.document_blob FOR SELECT TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND scan_state='CLEAN');
CREATE POLICY field_writer_download_intent ON finance.upload_intent FOR SELECT TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE FUNCTION finance.runtime_original_digest(t uuid,c uuid,a uuid,d uuid,i uuid)
RETURNS text LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT encode(b.sha256_digest,'hex') FROM finance.document_blob b
 JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(b.tenant_id,b.company_id,b.id)
 JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id)=(r.tenant_id,r.company_id,r.id)
 JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(r.tenant_id,r.company_id,r.dossier_id)
 JOIN finance.dossier_revision dr ON (dr.tenant_id,dr.company_id,dr.id,dr.dossier_id,dr.number)=(r.tenant_id,r.company_id,r.revision_id,d,ds.revision)
 WHERE (b.tenant_id,b.company_id,b.id)=(t,c,i) AND r.dossier_id=d
 AND b.scan_state='CLEAN' AND b.storage_backend='SYNTHETIC_DB_ONLY' AND u.state='FINALIZED'
 AND finance.field_access_allowed(t,c,d,a)
$$;
ALTER FUNCTION finance.runtime_original_digest(uuid,uuid,uuid,uuid,uuid) OWNER TO finance_field_review_writer;
REVOKE ALL ON FUNCTION finance.runtime_original_digest(uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_original_digest(uuid,uuid,uuid,uuid,uuid) TO finance_runtime;
CREATE FUNCTION finance.runtime_download_original(t uuid,c uuid,a uuid,d uuid,i uuid,cmd uuid,corr uuid)
RETURNS TABLE(sha256 text,mime text,bytes bytea)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE digest text;
BEGIN
 IF cmd IS NULL OR corr IS NULL THEN RAISE EXCEPTION 'download context required' USING ERRCODE='22023'; END IF;
 digest:=finance.runtime_original_digest(t,c,a,d,i);
 IF digest IS NULL THEN RETURN; END IF;
 RETURN QUERY SELECT encode(b.sha256_digest,'hex'),b.mime,b.synthetic_bytes
 FROM finance.document_blob b WHERE (b.tenant_id,b.company_id,b.id)=(t,c,i)
 AND b.scan_state='CLEAN' AND b.storage_backend='SYNTHETIC_DB_ONLY'
 AND encode(b.sha256_digest,'hex')=digest;
 IF FOUND THEN
  INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,i,'DOCUMENT_DOWNLOADED');
 END IF;
END $$;
ALTER FUNCTION finance.runtime_download_original(uuid,uuid,uuid,uuid,uuid,uuid,uuid) OWNER TO finance_field_review_writer;
REVOKE ALL ON FUNCTION finance.runtime_download_original(uuid,uuid,uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_download_original(uuid,uuid,uuid,uuid,uuid,uuid,uuid) TO finance_runtime;
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','DOCUMENT_DOWNLOADED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','RULE_SIMULATED','WARNING_ACKNOWLEDGED','DOSSIER_REVISED','DOSSIER_REOPENED','DOSSIER_CANCELLED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','POLICY_CREATED','POLICY_SUBMITTED','POLICY_PUBLISHED','POLICY_RETIRED','POLICY_REVOKED','POLICY_REVISED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
