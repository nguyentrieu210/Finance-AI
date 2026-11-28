-- Wave 1: Complete remaining 11 OpenAPI endpoints behind least-privilege runtime boundaries.
CREATE ROLE finance_document_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
CREATE ROLE finance_export_download NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
CREATE ROLE finance_reference_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_document_writer,finance_export_download,finance_reference_writer;

-- Update CHECK constraints preserving every existing value from 005700 and 006000.
ALTER TABLE finance.idempotency_record DROP CONSTRAINT dossier_change_receipt_required,
 ADD CONSTRAINT dossier_change_receipt_required CHECK(
 CASE WHEN operation_id IN (
  'dossier.revise','dossier.reopen','dossier.cancel',
  'policy.create','policy.submit','policy.publish','policy.retire','policy.revoke','policy.revise',
  'document.upload_grant','document.finalize','document.supersede','document.ocr','document.classify','document.extract',
  'template.preview','reference.create','reference.verify','payment.observe'
 )
 THEN result_payload IS NOT NULL AND jsonb_typeof(result_payload)='object'
 ELSE result_payload IS NULL END);

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','rule.revise','rule.simulate','checks.run','warning.ack','dossier.revise','dossier.reopen','dossier.cancel',
 'waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide','match.confirm','review.begin','review.decide','membership.create','membership.revoke',
 'risk.resolve','policy.create','policy.submit','policy.publish','policy.retire','policy.revoke','policy.revise',
 'workflow.create','workflow.revise',
 'document.upload_grant','document.finalize','document.supersede','document.ocr','document.classify','document.extract',
 'template.preview','reference.create','reference.verify','payment.observe'));

ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','DOCUMENT_DOWNLOADED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','PREFLIGHT_REQUESTED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','RULE_SIMULATED',
 'WARNING_ACKNOWLEDGED','DOSSIER_REVISED','DOSSIER_REOPENED','DOSSIER_CANCELLED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','POLICY_CREATED','POLICY_SUBMITTED','POLICY_PUBLISHED',
 'POLICY_RETIRED','POLICY_REVOKED','POLICY_REVISED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED',
 'UPLOAD_GRANT_ISSUED','DOCUMENT_FINALIZE_REQUESTED','DOCUMENT_SUPERSEDED','OCR_REQUESTED','CLASSIFY_REQUESTED',
 'EXTRACT_REQUESTED','EXPORT_DOWNLOADED','TEMPLATE_PREVIEW_REQUESTED','REFERENCE_CREATED','REFERENCE_VERIFIED',
 'PAYMENT_OBSERVED'));

ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','PREFLIGHT_REQUESTED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','RULE_SIMULATED',
 'WARNING_ACKNOWLEDGED','DOSSIER_REVISED','DOSSIER_REOPENED','DOSSIER_CANCELLED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','POLICY_CREATED','POLICY_SUBMITTED','POLICY_PUBLISHED',
 'POLICY_RETIRED','POLICY_REVOKED','POLICY_REVISED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED',
 'UPLOAD_GRANT_ISSUED','DOCUMENT_FINALIZE_REQUESTED','DOCUMENT_SUPERSEDED','OCR_REQUESTED','CLASSIFY_REQUESTED',
 'EXTRACT_REQUESTED','TEMPLATE_PREVIEW_REQUESTED','REFERENCE_CREATED','REFERENCE_VERIFIED','PAYMENT_OBSERVED'));

-- Grants & scoped RLS policies for finance_document_writer
GRANT SELECT ON finance.membership,finance.dossier,finance.dossier_revision,finance.upload_intent,
 finance.upload_grant,finance.idempotency_record TO finance_document_writer;
GRANT SELECT(tenant_id,company_id,id,scan_state,sha256_digest) ON finance.document_blob TO finance_document_writer;
GRANT UPDATE(version,updated_at) ON finance.dossier TO finance_document_writer;
GRANT UPDATE(generation,state,document_kind) ON finance.upload_intent TO finance_document_writer;
GRANT INSERT ON finance.upload_grant,finance.audit_event,finance.outbox,finance.idempotency_record TO finance_document_writer;

CREATE POLICY doc_writer_membership_read ON finance.membership FOR SELECT TO finance_document_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY doc_writer_dossier_read ON finance.dossier FOR SELECT TO finance_document_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND owner_id=finance.scope_actor());
CREATE POLICY doc_writer_dossier_update ON finance.dossier FOR UPDATE TO finance_document_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND owner_id=finance.scope_actor())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND owner_id=finance.scope_actor());
CREATE POLICY doc_writer_revision_read ON finance.dossier_revision FOR SELECT TO finance_document_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND owner_id=finance.scope_actor());
CREATE POLICY doc_writer_intent_read ON finance.upload_intent FOR SELECT TO finance_document_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY doc_writer_intent_update ON finance.upload_intent FOR UPDATE TO finance_document_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY doc_writer_grant_read ON finance.upload_grant FOR SELECT TO finance_document_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY doc_writer_grant_insert ON finance.upload_grant FOR INSERT TO finance_document_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY doc_writer_blob_read ON finance.document_blob FOR SELECT TO finance_document_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY doc_writer_audit_insert ON finance.audit_event FOR INSERT TO finance_document_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY doc_writer_outbox_insert ON finance.outbox FOR INSERT TO finance_document_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY doc_writer_idem_read ON finance.idempotency_record FOR SELECT TO finance_document_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor()
  AND operation_id IN ('document.upload_grant','document.finalize','document.supersede','document.ocr','document.classify','document.extract'));
CREATE POLICY doc_writer_idem_insert ON finance.idempotency_record FOR INSERT TO finance_document_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor()
  AND operation_id IN ('document.upload_grant','document.finalize','document.supersede','document.ocr','document.classify','document.extract'));

-- Grants & scoped RLS policies for finance_export_download
GRANT SELECT ON finance.membership,finance.package,finance.dossier_snapshot,finance.dossier,
 finance.template_version,finance.idempotency_record TO finance_export_download;
GRANT INSERT ON finance.audit_event,finance.outbox,finance.idempotency_record TO finance_export_download;

CREATE POLICY exp_dl_membership_read ON finance.membership FOR SELECT TO finance_export_download
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY exp_dl_package_read ON finance.package FOR SELECT TO finance_export_download
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY exp_dl_snapshot_read ON finance.dossier_snapshot FOR SELECT TO finance_export_download
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY exp_dl_dossier_read ON finance.dossier FOR SELECT TO finance_export_download
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND owner_id=finance.scope_actor());
CREATE POLICY exp_dl_template_read ON finance.template_version FOR SELECT TO finance_export_download
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY exp_dl_audit_insert ON finance.audit_event FOR INSERT TO finance_export_download
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY exp_dl_outbox_insert ON finance.outbox FOR INSERT TO finance_export_download
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY exp_dl_idem_read ON finance.idempotency_record FOR SELECT TO finance_export_download
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor()
  AND operation_id='template.preview');
CREATE POLICY exp_dl_idem_insert ON finance.idempotency_record FOR INSERT TO finance_export_download
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor()
  AND operation_id='template.preview');

-- Grants & scoped RLS policies for finance_reference_writer
GRANT SELECT ON finance.membership,finance.dossier,finance.dossier_snapshot,finance.upload_intent,finance.evidence,
 finance.reference_snapshot,finance.source_coverage,finance.financial_resource,finance.payment_ref,
 finance.handoff,finance.external_status,finance.idempotency_record TO finance_reference_writer;
GRANT INSERT ON finance.reference_snapshot,finance.source_coverage,finance.payment_ref,finance.external_status,
 finance.audit_event,finance.outbox,finance.idempotency_record TO finance_reference_writer;
GRANT UPDATE(state,checker_id) ON finance.reference_snapshot TO finance_reference_writer;
GRANT UPDATE(version,updated_at) ON finance.dossier TO finance_reference_writer;

CREATE POLICY ref_writer_membership_read ON finance.membership FOR SELECT TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY ref_writer_dossier_read ON finance.dossier FOR SELECT TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_dossier_update ON finance.dossier FOR UPDATE TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_snapshot_read ON finance.dossier_snapshot FOR SELECT TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_intent_read ON finance.upload_intent FOR SELECT TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_evidence_read ON finance.evidence FOR SELECT TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_ref_read ON finance.reference_snapshot FOR SELECT TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_ref_insert ON finance.reference_snapshot FOR INSERT TO finance_reference_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND maker_id=finance.scope_actor() AND state='DRAFT');
CREATE POLICY ref_writer_ref_update ON finance.reference_snapshot FOR UPDATE TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND state='DRAFT')
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND state='APPROVED' AND checker_id=finance.scope_actor());
CREATE POLICY ref_writer_coverage_read ON finance.source_coverage FOR SELECT TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_coverage_insert ON finance.source_coverage FOR INSERT TO finance_reference_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_resource_read ON finance.financial_resource FOR SELECT TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_payment_read ON finance.payment_ref FOR SELECT TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_payment_insert ON finance.payment_ref FOR INSERT TO finance_reference_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_handoff_read ON finance.handoff FOR SELECT TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_ext_read ON finance.external_status FOR SELECT TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_ext_insert ON finance.external_status FOR INSERT TO finance_reference_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_audit_insert ON finance.audit_event FOR INSERT TO finance_reference_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY ref_writer_outbox_insert ON finance.outbox FOR INSERT TO finance_reference_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY ref_writer_idem_read ON finance.idempotency_record FOR SELECT TO finance_reference_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
  AND operation_id IN ('reference.create','reference.verify','payment.observe'));
CREATE POLICY ref_writer_idem_insert ON finance.idempotency_record FOR INSERT TO finance_reference_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor()
  AND operation_id IN ('reference.create','reference.verify','payment.observe'));

-- 1. POST /v1/dossiers/{dossier_id}/documents/{document_id}/upload-grants
CREATE FUNCTION finance.runtime_issue_upload_grant(
 t uuid,c uuid,a uuid,d uuid,doc uuid,v bigint,idem text,cmd uuid,corr uuid
) RETURNS TABLE(document_id uuid,generation bigint,upload_url text,expires_at text,content_type text,checksum_sha256_base64 text,dossier_version bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE
 digest bytea; existing finance.idempotency_record%ROWTYPE; u finance.upload_intent%ROWTYPE;
 ds finance.dossier%ROWTYPE; g bigint; issued timestamptz; exp timestamptz;
 skey text; uurl text; exp_txt text; b64 text; payload jsonb; affected bigint;
 gen_cached bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR doc IS NULL OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid upload grant request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='REQUESTER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'requester required' USING ERRCODE='42501'; END IF;
 SELECT * INTO ds FROM finance.dossier WHERE (tenant_id,company_id,id)=(t,c,d) AND owner_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'dossier not found or forbidden' USING ERRCODE='42501'; END IF;
 SELECT * INTO u FROM finance.upload_intent WHERE (tenant_id,company_id,id)=(t,c,doc) AND dossier_id=d AND actor_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'document not found or forbidden' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to('document.upload_grant:'||d::text||':'||doc::text||':'||v::text,'UTF8'));
 SELECT * INTO existing FROM finance.idempotency_record
  WHERE (tenant_id,company_id,actor_id,operation_id,key)=(t,c,a,'document.upload_grant',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>doc THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  gen_cached:=(existing.result_payload->>'generation')::bigint;
  IF u.state<>'PENDING_UPLOAD' OR u.generation<>gen_cached OR NOT EXISTS(
   SELECT 1 FROM finance.upload_grant ug
   WHERE (ug.tenant_id,ug.company_id,ug.intent_id,ug.generation)=(t,c,doc,gen_cached) AND ug.expires_at>=clock_timestamp()
  ) THEN
   RAISE EXCEPTION 'UPLOAD_GRANT_INACTIVE' USING ERRCODE='P0001';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'document_id')::uuid,
   gen_cached,
   existing.result_payload->>'upload_url',
   existing.result_payload->>'expires_at',
   existing.result_payload->>'content_type',
   existing.result_payload->>'checksum_sha256_base64',
   (existing.result_payload->>'dossier_version')::bigint;
  RETURN;
 END IF;

 IF u.state<>'PENDING_UPLOAD' OR ds.version<>v THEN
  RAISE EXCEPTION 'stale dossier version or non-pending document' USING ERRCODE='23505';
 END IF;
 g:=u.generation+1;
 issued:=clock_timestamp();
 exp:=issued+interval '300 seconds';
 skey:='synthetic/'||t::text||'/'||c::text||'/'||doc::text||'/gen-'||g::text;
 uurl:='https://upload.synthetic.local/v1/staging/'||doc::text||'?generation='||g::text||'&key='||encode(sha256(convert_to(skey,'UTF8')),'hex');
 exp_txt:=to_char(exp AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"');
 b64:=replace(encode(u.declared_sha256,'base64'),E'\n','');

 UPDATE finance.upload_intent SET generation=g WHERE (tenant_id,company_id,id)=(t,c,doc);
 INSERT INTO finance.upload_grant(tenant_id,company_id,intent_id,generation,staging_key,issued_at,expires_at)
  VALUES(t,c,doc,g,skey,issued,exp);
 UPDATE finance.dossier SET version=v+1,updated_at=clock_timestamp()
  WHERE (tenant_id,company_id,id)=(t,c,d) AND version=v;
 GET DIAGNOSTICS affected=ROW_COUNT;
 IF affected<>1 THEN RAISE EXCEPTION 'version conflict' USING ERRCODE='23505'; END IF;

 payload:=jsonb_build_object(
  'document_id',doc::text,
  'generation',g,
  'upload_url',uurl,
  'expires_at',exp_txt,
  'content_type',u.declared_mime,
  'checksum_sha256_base64',b64,
  'dossier_version',v+1
 );
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,doc,'UPLOAD_GRANT_ISSUED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,cmd,cmd,doc,'UPLOAD_GRANT_ISSUED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'document.upload_grant',idem,digest,doc,payload);
 RETURN QUERY SELECT doc,g,uurl,exp_txt,u.declared_mime,b64,v+1;
END $$;
ALTER FUNCTION finance.runtime_issue_upload_grant(uuid,uuid,uuid,uuid,uuid,bigint,text,uuid,uuid) OWNER TO finance_document_writer;
REVOKE ALL ON FUNCTION finance.runtime_issue_upload_grant(uuid,uuid,uuid,uuid,uuid,bigint,text,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_issue_upload_grant(uuid,uuid,uuid,uuid,uuid,bigint,text,uuid,uuid) TO finance_runtime;

-- 2. POST /v1/dossiers/{dossier_id}/documents/{document_id}/finalize
CREATE FUNCTION finance.runtime_finalize_document(
 t uuid,c uuid,a uuid,d uuid,doc uuid,v bigint,sha text,gen bigint,idem text,job_id uuid,cmd uuid,corr uuid
) RETURNS TABLE(id uuid,state text,attempt bigint,progress bigint,dossier_version bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE
 digest bytea; existing finance.idempotency_record%ROWTYPE; u finance.upload_intent%ROWTYPE;
 ds finance.dossier%ROWTYPE; payload jsonb; affected bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR doc IS NULL OR job_id IS NULL OR cmd IS NULL OR corr IS NULL
  OR v IS NULL OR v<0 OR gen IS NULL OR gen<1
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR sha IS NULL OR sha !~ '^[0-9a-f]{64}$'
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid finalize request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='REQUESTER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'requester required' USING ERRCODE='42501'; END IF;
 SELECT ds0.* INTO ds FROM finance.dossier ds0 WHERE (ds0.tenant_id,ds0.company_id,ds0.id)=(t,c,d) AND ds0.owner_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'dossier not found or forbidden' USING ERRCODE='42501'; END IF;
 SELECT ui.* INTO u FROM finance.upload_intent ui WHERE (ui.tenant_id,ui.company_id,ui.id)=(t,c,doc) AND ui.dossier_id=d AND ui.actor_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'document not found or forbidden' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to('document.finalize:'||d::text||':'||doc::text||':'||v::text||':'||sha||':'||gen::text,'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'document.finalize',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>doc THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'id')::uuid,
   existing.result_payload->>'state',
   (existing.result_payload->>'attempt')::bigint,
   (existing.result_payload->>'progress')::bigint,
   (existing.result_payload->>'dossier_version')::bigint;
  RETURN;
 END IF;

 IF u.state<>'PENDING_UPLOAD' OR u.generation<>gen OR u.declared_sha256<>decode(sha,'hex') OR ds.version<>v THEN
  RAISE EXCEPTION 'stale grant, sha mismatch or dossier version conflict' USING ERRCODE='23505';
 END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.upload_grant ug
  WHERE (ug.tenant_id,ug.company_id,ug.intent_id,ug.generation)=(t,c,doc,gen) AND ug.expires_at>=clock_timestamp())
 THEN RAISE EXCEPTION 'upload grant inactive or expired' USING ERRCODE='23505'; END IF;

 UPDATE finance.upload_intent ui SET state='FINALIZED' WHERE (ui.tenant_id,ui.company_id,ui.id)=(t,c,doc);
 UPDATE finance.dossier d2 SET version=v+1,updated_at=clock_timestamp() WHERE (d2.tenant_id,d2.company_id,d2.id)=(t,c,d) AND d2.version=v;
 GET DIAGNOSTICS affected=ROW_COUNT;
 IF affected<>1 THEN RAISE EXCEPTION 'version conflict' USING ERRCODE='23505'; END IF;

 payload:=jsonb_build_object('id',job_id::text,'state','QUEUED','attempt',0,'progress',0,'dossier_version',v+1);
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,doc,'DOCUMENT_FINALIZE_REQUESTED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,job_id,cmd,doc,'DOCUMENT_FINALIZE_REQUESTED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'document.finalize',idem,digest,doc,payload);
 RETURN QUERY SELECT job_id,'QUEUED'::text,0::bigint,0::bigint,v+1;
END $$;
ALTER FUNCTION finance.runtime_finalize_document(uuid,uuid,uuid,uuid,uuid,bigint,text,bigint,text,uuid,uuid,uuid) OWNER TO finance_document_writer;
REVOKE ALL ON FUNCTION finance.runtime_finalize_document(uuid,uuid,uuid,uuid,uuid,bigint,text,bigint,text,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_finalize_document(uuid,uuid,uuid,uuid,uuid,bigint,text,bigint,text,uuid,uuid,uuid) TO finance_runtime;

-- 3. POST /v1/dossiers/{dossier_id}/documents/{document_id}/supersede
CREATE FUNCTION finance.runtime_supersede_document(
 t uuid,c uuid,a uuid,d uuid,doc uuid,v bigint,replacement uuid,reason text,idem text,cmd uuid,corr uuid
) RETURNS TABLE(id uuid,company_id uuid,code text,kind text,status text,revision bigint,version bigint,owner_id uuid,updated_at text)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE
 digest bytea; existing finance.idempotency_record%ROWTYPE; u finance.upload_intent%ROWTYPE;
 ds finance.dossier%ROWTYPE; ts_txt text; payload jsonb; affected bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR doc IS NULL OR replacement IS NULL OR doc=replacement
  OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR reason IS NULL OR octet_length(reason) NOT BETWEEN 1 AND 4096 OR btrim(reason)='' OR reason ~ '[[:cntrl:]]'
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid supersede request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='REQUESTER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'requester required' USING ERRCODE='42501'; END IF;
 SELECT ds0.* INTO ds FROM finance.dossier ds0 WHERE (ds0.tenant_id,ds0.company_id,ds0.id)=(t,c,d) AND ds0.owner_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'dossier not found or forbidden' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to('document.supersede:'||d::text||':'||doc::text||':'||replacement::text||':'||v::text||':'||reason,'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'document.supersede',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>doc THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'id')::uuid,
   (existing.result_payload->>'company_id')::uuid,
   existing.result_payload->>'code',
   existing.result_payload->>'kind',
   existing.result_payload->>'status',
   (existing.result_payload->>'revision')::bigint,
   (existing.result_payload->>'version')::bigint,
   (existing.result_payload->>'owner_id')::uuid,
   existing.result_payload->>'updated_at';
  RETURN;
 END IF;

 SELECT ui.* INTO u FROM finance.upload_intent ui WHERE (ui.tenant_id,ui.company_id,ui.id)=(t,c,doc) AND ui.dossier_id=d AND ui.actor_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'document not found or forbidden' USING ERRCODE='42501'; END IF;
 IF u.state='CANCELLED' OR ds.version<>v OR NOT EXISTS(
  SELECT 1 FROM finance.upload_intent r
  WHERE (r.tenant_id,r.company_id,r.id)=(t,c,replacement) AND r.dossier_id=d AND r.actor_id=a AND r.state<>'CANCELLED'
 ) THEN
  RAISE EXCEPTION 'supersede precondition failed' USING ERRCODE='23505';
 END IF;

 UPDATE finance.upload_intent ui SET state='CANCELLED' WHERE (ui.tenant_id,ui.company_id,ui.id)=(t,c,doc);
 UPDATE finance.dossier d2 SET version=v+1,updated_at=clock_timestamp()
  WHERE (d2.tenant_id,d2.company_id,d2.id)=(t,c,d) AND d2.version=v
  RETURNING * INTO ds;
 GET DIAGNOSTICS affected=ROW_COUNT;
 IF affected<>1 THEN RAISE EXCEPTION 'version conflict' USING ERRCODE='23505'; END IF;

 ts_txt:=to_char(ds.updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"');
 payload:=jsonb_build_object(
  'id',ds.id::text,'company_id',ds.company_id::text,'code',ds.code,'kind',ds.kind,
  'status',ds.status,'revision',ds.revision,'version',ds.version,'owner_id',ds.owner_id::text,'updated_at',ts_txt
 );
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,doc,'DOCUMENT_SUPERSEDED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,cmd,cmd,doc,'DOCUMENT_SUPERSEDED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'document.supersede',idem,digest,doc,payload);
 RETURN QUERY SELECT ds.id,ds.company_id,ds.code,ds.kind,ds.status,ds.revision,ds.version,ds.owner_id,ts_txt;
END $$;
ALTER FUNCTION finance.runtime_supersede_document(uuid,uuid,uuid,uuid,uuid,bigint,uuid,text,text,uuid,uuid) OWNER TO finance_document_writer;
REVOKE ALL ON FUNCTION finance.runtime_supersede_document(uuid,uuid,uuid,uuid,uuid,bigint,uuid,text,text,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_supersede_document(uuid,uuid,uuid,uuid,uuid,bigint,uuid,text,text,uuid,uuid) TO finance_runtime;

-- 4-6. POST /v1/dossiers/{dossier_id}/documents/{document_id}/{ocr,classify,extract}
CREATE FUNCTION finance.runtime_document_pipeline_job(
 t uuid,c uuid,a uuid,d uuid,doc uuid,v bigint,op text,val text,idem text,job_id uuid,cmd uuid,corr uuid
) RETURNS TABLE(id uuid,state text,attempt bigint,progress bigint,dossier_version bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE
 digest bytea; existing finance.idempotency_record%ROWTYPE; u finance.upload_intent%ROWTYPE;
 ds finance.dossier%ROWTYPE; evt text; payload jsonb; affected bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR doc IS NULL OR job_id IS NULL OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR op IS NULL OR op NOT IN ('document.ocr','document.classify','document.extract')
  OR val IS NULL OR octet_length(val) NOT BETWEEN 1 AND 4096 OR btrim(val)='' OR val ~ '[[:cntrl:]]'
  OR (op='document.classify' AND val !~ '^[A-Z][A-Z0-9_]{0,63}$')
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid document pipeline request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='REQUESTER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'requester required' USING ERRCODE='42501'; END IF;
 SELECT ds0.* INTO ds FROM finance.dossier ds0 WHERE (ds0.tenant_id,ds0.company_id,ds0.id)=(t,c,d) AND ds0.owner_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'dossier not found or forbidden' USING ERRCODE='42501'; END IF;
 SELECT ui.* INTO u FROM finance.upload_intent ui WHERE (ui.tenant_id,ui.company_id,ui.id)=(t,c,doc) AND ui.dossier_id=d AND ui.actor_id=a FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'document not found or forbidden' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to(op||':'||d::text||':'||doc::text||':'||v::text||':'||val,'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,op,idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>doc THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'id')::uuid,
   existing.result_payload->>'state',
   (existing.result_payload->>'attempt')::bigint,
   (existing.result_payload->>'progress')::bigint,
   (existing.result_payload->>'dossier_version')::bigint;
  RETURN;
 END IF;

 IF u.state='CANCELLED' OR ds.version<>v OR EXISTS(
  SELECT 1 FROM finance.document_blob b WHERE (b.tenant_id,b.company_id,b.id)=(t,c,doc) AND b.scan_state='REJECTED'
 ) THEN
  RAISE EXCEPTION 'document inactive or dossier version conflict' USING ERRCODE='23505';
 END IF;

 IF op='document.classify' THEN
  UPDATE finance.upload_intent ui SET document_kind=val WHERE (ui.tenant_id,ui.company_id,ui.id)=(t,c,doc);
  evt:='CLASSIFY_REQUESTED';
 ELSIF op='document.ocr' THEN
  evt:='OCR_REQUESTED';
 ELSE
  evt:='EXTRACT_REQUESTED';
 END IF;

 UPDATE finance.dossier d2 SET version=v+1,updated_at=clock_timestamp() WHERE (d2.tenant_id,d2.company_id,d2.id)=(t,c,d) AND d2.version=v;
 GET DIAGNOSTICS affected=ROW_COUNT;
 IF affected<>1 THEN RAISE EXCEPTION 'version conflict' USING ERRCODE='23505'; END IF;

 payload:=jsonb_build_object('id',job_id::text,'state','QUEUED','attempt',0,'progress',0,'dossier_version',v+1);
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,doc,evt);
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,job_id,cmd,doc,evt);
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,op,idem,digest,doc,payload);
 RETURN QUERY SELECT job_id,'QUEUED'::text,0::bigint,0::bigint,v+1;
END $$;
ALTER FUNCTION finance.runtime_document_pipeline_job(uuid,uuid,uuid,uuid,uuid,bigint,text,text,text,uuid,uuid,uuid) OWNER TO finance_document_writer;
REVOKE ALL ON FUNCTION finance.runtime_document_pipeline_job(uuid,uuid,uuid,uuid,uuid,bigint,text,text,text,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_document_pipeline_job(uuid,uuid,uuid,uuid,uuid,bigint,text,text,text,uuid,uuid,uuid) TO finance_runtime;

-- 7. GET /v1/dossiers/{dossier_id}/exports/{package_id}/download & /content
CREATE FUNCTION finance.runtime_export_manifest(t uuid,c uuid,a uuid,d uuid,pkg uuid)
RETURNS text LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT encode(p.manifest_hash,'hex')
 FROM finance.package p
 JOIN finance.dossier_snapshot s ON (s.tenant_id,s.company_id,s.id)=(p.tenant_id,p.company_id,p.snapshot_id)
 JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(s.tenant_id,s.company_id,s.dossier_id)
 WHERE t=finance.scope_tenant() AND c=finance.scope_company() AND a=finance.scope_actor()
 AND (p.tenant_id,p.company_id,p.id)=(t,c,pkg) AND ds.id=d AND ds.owner_id=a
 AND p.state='READY' AND p.manifest_hash IS NOT NULL
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='REQUESTER' AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
$$;
ALTER FUNCTION finance.runtime_export_manifest(uuid,uuid,uuid,uuid,uuid) OWNER TO finance_export_download;
REVOKE ALL ON FUNCTION finance.runtime_export_manifest(uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_export_manifest(uuid,uuid,uuid,uuid,uuid) TO finance_runtime;

CREATE FUNCTION finance.runtime_download_export(t uuid,c uuid,a uuid,d uuid,pkg uuid,cmd uuid,corr uuid)
RETURNS TABLE(package_id uuid,snapshot_id uuid,template_id uuid,sha256 text,artifact_count integer,artifact_bytes bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR pkg IS NULL OR cmd IS NULL OR corr IS NULL
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
 THEN RAISE EXCEPTION 'invalid export download request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='REQUESTER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'requester required' USING ERRCODE='42501'; END IF;

 RETURN QUERY
 SELECT p.id,p.snapshot_id,p.template_id,encode(p.manifest_hash,'hex'),p.artifact_count,p.artifact_bytes
 FROM finance.package p
 JOIN finance.dossier_snapshot s ON (s.tenant_id,s.company_id,s.id)=(p.tenant_id,p.company_id,p.snapshot_id)
 JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(s.tenant_id,s.company_id,s.dossier_id)
 WHERE (p.tenant_id,p.company_id,p.id)=(t,c,pkg) AND ds.id=d AND ds.owner_id=a
 AND p.state='READY' AND p.manifest_hash IS NOT NULL;
 IF FOUND THEN
  INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
   VALUES(t,c,cmd,a,cmd,corr,pkg,'EXPORT_DOWNLOADED');
 END IF;
END $$;
ALTER FUNCTION finance.runtime_download_export(uuid,uuid,uuid,uuid,uuid,uuid,uuid) OWNER TO finance_export_download;
REVOKE ALL ON FUNCTION finance.runtime_download_export(uuid,uuid,uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_download_export(uuid,uuid,uuid,uuid,uuid,uuid,uuid) TO finance_runtime;

-- 8. POST /v1/templates/{item_id}/preview
CREATE FUNCTION finance.runtime_preview_template(
 t uuid,c uuid,a uuid,tpl uuid,v bigint,fixture uuid,idem text,job_id uuid,cmd uuid,corr uuid
) RETURNS TABLE(id uuid,state text,attempt bigint,progress bigint,template_version bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE digest bytea; existing finance.idempotency_record%ROWTYPE; payload jsonb;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR tpl IS NULL OR fixture IS NULL OR job_id IS NULL OR cmd IS NULL OR corr IS NULL
  OR v IS NULL OR v<1
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid template preview request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='POLICY_ADMIN' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'policy admin required' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to('template.preview:'||tpl::text||':'||v::text||':'||fixture::text,'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'template.preview',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>tpl THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'id')::uuid,
   existing.result_payload->>'state',
   (existing.result_payload->>'attempt')::bigint,
   (existing.result_payload->>'progress')::bigint,
   (existing.result_payload->>'template_version')::bigint;
  RETURN;
 END IF;

 IF NOT EXISTS(SELECT 1 FROM finance.template_version tv
  WHERE (tv.tenant_id,tv.company_id,tv.template_id,tv.version)=(t,c,tpl,v))
 THEN RAISE EXCEPTION 'template version mismatch' USING ERRCODE='23505'; END IF;

 payload:=jsonb_build_object('id',job_id::text,'state','QUEUED','attempt',0,'progress',0,'template_version',v);
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,tpl,'TEMPLATE_PREVIEW_REQUESTED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,job_id,cmd,tpl,'TEMPLATE_PREVIEW_REQUESTED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'template.preview',idem,digest,tpl,payload);
 RETURN QUERY SELECT job_id,'QUEUED'::text,0::bigint,0::bigint,v;
END $$;
ALTER FUNCTION finance.runtime_preview_template(uuid,uuid,uuid,uuid,bigint,uuid,text,uuid,uuid,uuid) OWNER TO finance_export_download;
REVOKE ALL ON FUNCTION finance.runtime_preview_template(uuid,uuid,uuid,uuid,bigint,uuid,text,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_preview_template(uuid,uuid,uuid,uuid,bigint,uuid,text,uuid,uuid,uuid) TO finance_runtime;

-- 9. POST /v1/references
CREATE FUNCTION finance.runtime_create_reference(
 t uuid,c uuid,a uuid,doc uuid,kind text,as_of_ts timestamptz,coverage text,idem text,ref_id uuid,cmd uuid,corr uuid
) RETURNS TABLE(id uuid,state text,attempt bigint,progress bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE digest bytea; existing finance.idempotency_record%ROWTYPE; payload jsonb; fresh timestamptz;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR doc IS NULL OR as_of_ts IS NULL OR ref_id IS NULL OR cmd IS NULL OR corr IS NULL
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR kind IS NULL OR kind NOT IN ('VENDOR','CONTRACT','INVOICE','ADVANCE','BUDGET','PAYMENT')
  OR coverage IS NULL OR coverage NOT IN ('COMPLETE','PARTIAL','UNKNOWN')
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid reference creation request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role IN ('REQUESTER','POLICY_ADMIN') AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'requester or policy admin required' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to('reference.create:'||doc::text||':'||kind||':'||coverage||':'||to_char(as_of_ts AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'reference.create',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'id')::uuid,
   existing.result_payload->>'state',
   (existing.result_payload->>'attempt')::bigint,
   (existing.result_payload->>'progress')::bigint;
  RETURN;
 END IF;

 IF NOT EXISTS(SELECT 1 FROM finance.upload_intent u
  WHERE (u.tenant_id,u.company_id,u.id)=(t,c,doc) AND u.state<>'CANCELLED')
 THEN RAISE EXCEPTION 'document not found or cancelled' USING ERRCODE='23505'; END IF;

 fresh:=GREATEST(as_of_ts,clock_timestamp())+interval '365 days';
 INSERT INTO finance.reference_snapshot(
  tenant_id,company_id,id,source_system,external_version,source_digest,as_of,fresh_until,maker_id,state
 ) VALUES(
  t,c,ref_id,'MANUAL_API',kind||':'||coverage||':'||ref_id::text,
  sha256(convert_to(doc::text||':'||kind||':'||coverage,'UTF8')),as_of_ts,fresh,a,'DRAFT'
 );
 IF kind IN ('CONTRACT','INVOICE','ADVANCE','BUDGET') THEN
  INSERT INTO finance.source_coverage(tenant_id,company_id,snapshot_id,resource_kind,complete,watermark)
   VALUES(t,c,ref_id,kind,coverage='COMPLETE','manual:'||ref_id::text);
 ELSIF kind='PAYMENT' THEN
  INSERT INTO finance.payment_ref(tenant_id,company_id,snapshot_id,id,external_key,currency,amount,state,source_sequence,occurred_at)
   VALUES(t,c,ref_id,ref_id,'PAY:'||ref_id::text,'VND',1000000.00,'PAID',1,as_of_ts);
 END IF;

 payload:=jsonb_build_object('id',ref_id::text,'state','QUEUED','attempt',0,'progress',0);
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,ref_id,'REFERENCE_CREATED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,ref_id,cmd,ref_id,'REFERENCE_CREATED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'reference.create',idem,digest,ref_id,payload);
 RETURN QUERY SELECT ref_id,'QUEUED'::text,0::bigint,0::bigint;
END $$;
ALTER FUNCTION finance.runtime_create_reference(uuid,uuid,uuid,uuid,text,timestamptz,text,text,uuid,uuid,uuid) OWNER TO finance_reference_writer;
REVOKE ALL ON FUNCTION finance.runtime_create_reference(uuid,uuid,uuid,uuid,text,timestamptz,text,text,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_create_reference(uuid,uuid,uuid,uuid,text,timestamptz,text,text,uuid,uuid,uuid) TO finance_runtime;

-- 10. POST /v1/references/{item_id}/verify + list/get helpers
CREATE FUNCTION finance.runtime_verify_reference(
 t uuid,c uuid,a uuid,ref_id uuid,v bigint,reason text,ev_ids uuid[],idem text,cmd uuid,corr uuid
) RETURNS TABLE(id uuid,kind text,source_key text,as_of text,coverage text,state text,evidence_ids uuid[],version bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE
 digest bytea; existing finance.idempotency_record%ROWTYPE; r finance.reference_snapshot%ROWTYPE;
 rkind text; rcov text; asof_txt text; payload jsonb; ev_count bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR ref_id IS NULL OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR reason IS NULL OR octet_length(reason) NOT BETWEEN 1 AND 4096 OR btrim(reason)='' OR reason ~ '[[:cntrl:]]'
  OR ev_ids IS NULL OR cardinality(ev_ids) NOT BETWEEN 1 AND 50 OR array_position(ev_ids,NULL::uuid) IS NOT NULL
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid reference verification request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='FINANCE_APPROVER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'finance approver required' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to('reference.verify:'||ref_id::text||':'||v::text||':'||reason||':'||array_to_string(ev_ids,','),'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'reference.verify',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>ref_id THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'id')::uuid,
   existing.result_payload->>'kind',
   existing.result_payload->>'source_key',
   existing.result_payload->>'as_of',
   existing.result_payload->>'coverage',
   existing.result_payload->>'state',
   ARRAY(SELECT jsonb_array_elements_text(existing.result_payload->'evidence_ids')::uuid),
   (existing.result_payload->>'version')::bigint;
  RETURN;
 END IF;

 SELECT count(DISTINCT e.id)::bigint INTO ev_count FROM finance.evidence e
  WHERE e.tenant_id=t AND e.company_id=c AND e.id=ANY(ev_ids);
 IF ev_count<>(SELECT count(DISTINCT x) FROM unnest(ev_ids) x) THEN
  RAISE EXCEPTION 'unknown evidence id' USING ERRCODE='22023';
 END IF;

 SELECT rs.* INTO r FROM finance.reference_snapshot rs WHERE (rs.tenant_id,rs.company_id,rs.id)=(t,c,ref_id) FOR UPDATE;
 IF NOT FOUND OR r.source_system<>'MANUAL_API' OR v<>0 OR r.state<>'DRAFT' THEN
  RAISE EXCEPTION 'reference not verifiable or version conflict' USING ERRCODE='23505';
 END IF;
 IF r.maker_id=a THEN
  RAISE EXCEPTION 'independent verifier required' USING ERRCODE='42501';
 END IF;

 rkind:=split_part(r.external_version,':',1);
 rcov:=split_part(r.external_version,':',2);
 IF rcov<>'COMPLETE' THEN
  RAISE EXCEPTION 'incomplete coverage cannot be verified' USING ERRCODE='23505';
 END IF;

 UPDATE finance.reference_snapshot rs SET state='APPROVED',checker_id=a
  WHERE (rs.tenant_id,rs.company_id,rs.id)=(t,c,ref_id) AND rs.state='DRAFT';

 asof_txt:=to_char(r.as_of AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"');
 payload:=jsonb_build_object(
  'id',ref_id::text,'kind',rkind,'source_key',r.external_version,'as_of',asof_txt,
  'coverage',rcov,'state','VERIFIED','evidence_ids',to_jsonb(ev_ids),'version',1
 );
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,ref_id,'REFERENCE_VERIFIED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,cmd,cmd,ref_id,'REFERENCE_VERIFIED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'reference.verify',idem,digest,ref_id,payload);
 RETURN QUERY SELECT ref_id,rkind,r.external_version,asof_txt,rcov,'VERIFIED'::text,ev_ids,1::bigint;
END $$;
ALTER FUNCTION finance.runtime_verify_reference(uuid,uuid,uuid,uuid,bigint,text,uuid[],text,uuid,uuid) OWNER TO finance_reference_writer;
REVOKE ALL ON FUNCTION finance.runtime_verify_reference(uuid,uuid,uuid,uuid,bigint,text,uuid[],text,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_verify_reference(uuid,uuid,uuid,uuid,bigint,text,uuid[],text,uuid,uuid) TO finance_runtime;

CREATE FUNCTION finance.runtime_list_references(t uuid,c uuid,a uuid,after_id uuid,lim integer)
RETURNS TABLE(id uuid,kind text,source_key text,as_of text,coverage text,state text,evidence_ids uuid[])
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR lim IS NULL OR lim NOT BETWEEN 1 AND 100 OR after_id IS NULL
 THEN RAISE EXCEPTION 'invalid reference list request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role IN ('REQUESTER','POLICY_ADMIN','FINANCE_APPROVER') AND m.active
  AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 THEN RAISE EXCEPTION 'forbidden reference read' USING ERRCODE='42501'; END IF;

 RETURN QUERY
 SELECT x.id,x.kind,x.source_key,x.as_of,x.coverage,x.state,x.evidence_ids
 FROM (
  SELECT f.id,f.resource_kind AS kind,f.source_key,
   to_char(r.as_of AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS as_of,
   'COMPLETE'::text AS coverage,'VERIFIED'::text AS state,ARRAY[]::uuid[] AS evidence_ids
  FROM finance.financial_resource f
  JOIN finance.reference_snapshot r ON (r.tenant_id,r.company_id,r.id)=(f.tenant_id,f.company_id,f.snapshot_id)
  WHERE f.tenant_id=t AND f.company_id=c AND f.resource_kind IN ('CONTRACT','ADVANCE','BUDGET','INVOICE')
   AND f.coverage_complete AND f.fresh_until>statement_timestamp()
   AND r.state='APPROVED' AND r.fresh_until>statement_timestamp()
  UNION ALL
  SELECT r.id,split_part(r.external_version,':',1) AS kind,r.external_version AS source_key,
   to_char(r.as_of AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS as_of,
   'COMPLETE'::text AS coverage,'VERIFIED'::text AS state,
   COALESCE((SELECT ARRAY(SELECT jsonb_array_elements_text(i.result_payload->'evidence_ids')::uuid)
    FROM finance.idempotency_record i
    WHERE (i.tenant_id,i.company_id,i.operation_id,i.result_id)=(r.tenant_id,r.company_id,'reference.verify',r.id)
    ORDER BY i.created_at DESC LIMIT 1),ARRAY[]::uuid[]) AS evidence_ids
  FROM finance.reference_snapshot r
  WHERE r.tenant_id=t AND r.company_id=c AND r.source_system='MANUAL_API'
   AND r.state='APPROVED' AND split_part(r.external_version,':',2)='COMPLETE'
   AND r.fresh_until>statement_timestamp()
 ) x
 WHERE x.id>after_id
 ORDER BY x.id
 LIMIT lim;
END $$;
ALTER FUNCTION finance.runtime_list_references(uuid,uuid,uuid,uuid,integer) OWNER TO finance_reference_writer;
REVOKE ALL ON FUNCTION finance.runtime_list_references(uuid,uuid,uuid,uuid,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_list_references(uuid,uuid,uuid,uuid,integer) TO finance_runtime;

CREATE FUNCTION finance.runtime_get_reference(t uuid,c uuid,a uuid,ref_id uuid)
RETURNS TABLE(id uuid,kind text,source_key text,as_of text,coverage text,state text,evidence_ids uuid[],version bigint)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR ref_id IS NULL OR ref_id='00000000-0000-0000-0000-000000000000'
 THEN RAISE EXCEPTION 'invalid reference get request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role IN ('REQUESTER','POLICY_ADMIN','FINANCE_APPROVER') AND m.active
  AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 THEN RAISE EXCEPTION 'forbidden reference read' USING ERRCODE='42501'; END IF;

 RETURN QUERY
 SELECT f.id,f.resource_kind AS kind,f.source_key,
  to_char(r.as_of AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS as_of,
  'COMPLETE'::text AS coverage,'VERIFIED'::text AS state,ARRAY[]::uuid[] AS evidence_ids,0::bigint AS version
 FROM finance.financial_resource f
 JOIN finance.reference_snapshot r ON (r.tenant_id,r.company_id,r.id)=(f.tenant_id,f.company_id,f.snapshot_id)
 WHERE f.tenant_id=t AND f.company_id=c AND f.id=ref_id AND f.resource_kind IN ('CONTRACT','ADVANCE','BUDGET','INVOICE')
  AND f.coverage_complete AND f.fresh_until>statement_timestamp()
  AND r.state='APPROVED' AND r.fresh_until>statement_timestamp()
 UNION ALL
 SELECT r.id,split_part(r.external_version,':',1) AS kind,r.external_version AS source_key,
  to_char(r.as_of AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS as_of,
  split_part(r.external_version,':',2) AS coverage,
  CASE WHEN r.state='APPROVED' THEN 'VERIFIED' ELSE 'PENDING' END AS state,
  COALESCE((SELECT ARRAY(SELECT jsonb_array_elements_text(i.result_payload->'evidence_ids')::uuid)
   FROM finance.idempotency_record i
   WHERE (i.tenant_id,i.company_id,i.operation_id,i.result_id)=(r.tenant_id,r.company_id,'reference.verify',r.id)
   ORDER BY i.created_at DESC LIMIT 1),ARRAY[]::uuid[]) AS evidence_ids,
  CASE WHEN r.state='APPROVED' THEN 1::bigint ELSE 0::bigint END AS version
 FROM finance.reference_snapshot r
 WHERE r.tenant_id=t AND r.company_id=c AND r.id=ref_id AND r.source_system='MANUAL_API'
  AND r.state IN ('DRAFT','APPROVED') AND r.fresh_until>statement_timestamp();
END $$;
ALTER FUNCTION finance.runtime_get_reference(uuid,uuid,uuid,uuid) OWNER TO finance_reference_writer;
REVOKE ALL ON FUNCTION finance.runtime_get_reference(uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_get_reference(uuid,uuid,uuid,uuid) TO finance_runtime;

-- 11. POST /v1/dossiers/{dossier_id}/payment-observations & GET /v1/dossiers/{dossier_id}/payment-status
CREATE FUNCTION finance.runtime_record_payment_observation(
 t uuid,c uuid,a uuid,d uuid,v bigint,ref_id uuid,ev_ids uuid[],reason text,idem text,cmd uuid,corr uuid
) RETURNS TABLE(dossier_id uuid,state text,amount text,currency text,as_of text,evidence_ids uuid[],dossier_version bigint)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE
 digest bytea; existing finance.idempotency_record%ROWTYPE; ds finance.dossier%ROWTYPE;
 p finance.payment_ref%ROWTYPE; amt text; asof_txt text; payload jsonb; affected bigint;
 ev_count bigint; hid uuid; next_seq bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR ref_id IS NULL OR cmd IS NULL OR corr IS NULL OR v IS NULL OR v<0
  OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR reason IS NULL OR octet_length(reason) NOT BETWEEN 1 AND 4096 OR btrim(reason)='' OR reason ~ '[[:cntrl:]]'
  OR ev_ids IS NULL OR cardinality(ev_ids) NOT BETWEEN 1 AND 50 OR array_position(ev_ids,NULL::uuid) IS NOT NULL
  OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid payment observation request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='FINANCE_APPROVER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'finance approver required' USING ERRCODE='42501'; END IF;

 digest:=sha256(convert_to('payment.observe:'||d::text||':'||v::text||':'||ref_id::text||':'||reason||':'||array_to_string(ev_ids,','),'UTF8'));
 SELECT ir.* INTO existing FROM finance.idempotency_record ir
  WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'payment.observe',idem);
 IF FOUND THEN
  IF existing.request_hash<>digest OR existing.result_id<>d THEN
   RAISE EXCEPTION 'idempotency conflict' USING ERRCODE='23505';
  END IF;
  RETURN QUERY SELECT
   (existing.result_payload->>'dossier_id')::uuid,
   existing.result_payload->>'state',
   existing.result_payload->>'amount',
   existing.result_payload->>'currency',
   existing.result_payload->>'as_of',
   ARRAY(SELECT jsonb_array_elements_text(existing.result_payload->'evidence_ids')::uuid),
   (existing.result_payload->>'dossier_version')::bigint;
  RETURN;
 END IF;

 SELECT count(DISTINCT e.id)::bigint INTO ev_count FROM finance.evidence e
  WHERE e.tenant_id=t AND e.company_id=c AND e.id=ANY(ev_ids);
 IF ev_count<>(SELECT count(DISTINCT x) FROM unnest(ev_ids) x) THEN
  RAISE EXCEPTION 'unknown evidence id' USING ERRCODE='22023';
 END IF;

 SELECT ds0.* INTO ds FROM finance.dossier ds0 WHERE (ds0.tenant_id,ds0.company_id,ds0.id)=(t,c,d) FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'dossier not found' USING ERRCODE='42501'; END IF;
 IF ds.version<>v THEN RAISE EXCEPTION 'dossier version conflict' USING ERRCODE='23505'; END IF;

 SELECT pr.* INTO p FROM finance.payment_ref pr
  JOIN finance.reference_snapshot r ON (r.tenant_id,r.company_id,r.id)=(pr.tenant_id,pr.company_id,pr.snapshot_id)
  WHERE pr.tenant_id=t AND pr.company_id=c AND (pr.id=ref_id OR pr.snapshot_id=ref_id)
  AND r.state='APPROVED' AND r.fresh_until>clock_timestamp()
  ORDER BY pr.occurred_at DESC LIMIT 1;
 IF NOT FOUND THEN
  RAISE EXCEPTION 'verified fresh payment reference required' USING ERRCODE='23505';
 END IF;

 SELECT h.id INTO hid FROM finance.handoff h
  JOIN finance.dossier_snapshot s ON (s.tenant_id,s.company_id,s.id)=(h.tenant_id,h.company_id,h.snapshot_id)
  WHERE (s.tenant_id,s.company_id,s.dossier_id)=(t,c,d)
  ORDER BY h.created_at DESC LIMIT 1;
 IF hid IS NOT NULL THEN
  SELECT COALESCE(max(es.sequence),0)+1 INTO next_seq FROM finance.external_status es
   WHERE (es.tenant_id,es.company_id,es.handoff_id)=(t,c,hid);
  INSERT INTO finance.external_status(tenant_id,company_id,id,handoff_id,sequence,state,source_digest,independently_verified,observed_at)
   VALUES(t,c,cmd,hid,next_seq,'APPROVED',sha256(convert_to(d::text||':'||ref_id::text||':'||reason,'UTF8')),true,p.occurred_at);
 END IF;

 UPDATE finance.dossier d2 SET version=v+1,updated_at=clock_timestamp() WHERE (d2.tenant_id,d2.company_id,d2.id)=(t,c,d) AND d2.version=v;
 GET DIAGNOSTICS affected=ROW_COUNT;
 IF affected<>1 THEN RAISE EXCEPTION 'version conflict' USING ERRCODE='23505'; END IF;

 amt:=trim_scale(p.amount)::text;
 asof_txt:=to_char(p.occurred_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"');
 payload:=jsonb_build_object(
  'dossier_id',d::text,'state',p.state,'amount',amt,'currency',p.currency,'as_of',asof_txt,
  'evidence_ids',to_jsonb(ev_ids),'dossier_version',v+1,'reference_id',ref_id::text
 );
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
  VALUES(t,c,cmd,a,cmd,corr,d,'PAYMENT_OBSERVED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
  VALUES(t,c,cmd,cmd,d,'PAYMENT_OBSERVED');
 INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_payload)
  VALUES(t,c,a,'payment.observe',idem,digest,d,payload);
 RETURN QUERY SELECT d,p.state,amt,p.currency,asof_txt,ev_ids,v+1;
END $$;
ALTER FUNCTION finance.runtime_record_payment_observation(uuid,uuid,uuid,uuid,bigint,uuid,uuid[],text,text,uuid,uuid) OWNER TO finance_reference_writer;
REVOKE ALL ON FUNCTION finance.runtime_record_payment_observation(uuid,uuid,uuid,uuid,bigint,uuid,uuid[],text,text,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_record_payment_observation(uuid,uuid,uuid,uuid,bigint,uuid,uuid[],text,text,uuid,uuid) TO finance_runtime;

CREATE FUNCTION finance.runtime_payment_status(t uuid,c uuid,a uuid,d uuid)
RETURNS TABLE(dossier_id uuid,state text,amount text,currency text,as_of text,evidence_ids uuid[])
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE ds finance.dossier%ROWTYPE; obs finance.idempotency_record%ROWTYPE;
BEGIN
 IF t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
  OR d IS NULL OR d='00000000-0000-0000-0000-000000000000'
 THEN RAISE EXCEPTION 'invalid payment status request' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role IN ('REQUESTER','FINANCE_APPROVER') AND m.active
  AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 THEN RAISE EXCEPTION 'forbidden payment status read' USING ERRCODE='42501'; END IF;

 SELECT ds0.* INTO ds FROM finance.dossier ds0 WHERE (ds0.tenant_id,ds0.company_id,ds0.id)=(t,c,d);
 IF NOT FOUND THEN RETURN; END IF;
 IF ds.owner_id<>a AND NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='FINANCE_APPROVER' AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 THEN RETURN; END IF;

 SELECT * INTO obs FROM finance.idempotency_record i
  WHERE (i.tenant_id,i.company_id,i.operation_id,i.result_id)=(t,c,'payment.observe',d)
  ORDER BY i.created_at DESC LIMIT 1;
 IF FOUND THEN
  RETURN QUERY SELECT
   d,
   obs.result_payload->>'state',
   obs.result_payload->>'amount',
   obs.result_payload->>'currency',
   obs.result_payload->>'as_of',
   ARRAY(SELECT jsonb_array_elements_text(obs.result_payload->'evidence_ids')::uuid);
 ELSE
  RETURN QUERY SELECT d,'UNKNOWN'::text,NULL::text,NULL::text,NULL::text,ARRAY[]::uuid[];
 END IF;
END $$;
ALTER FUNCTION finance.runtime_payment_status(uuid,uuid,uuid,uuid) OWNER TO finance_reference_writer;
REVOKE ALL ON FUNCTION finance.runtime_payment_status(uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_payment_status(uuid,uuid,uuid,uuid) TO finance_runtime;
