-- E16: create drafts only from scoped immutable clean originals and persisted source evidence.
DROP TRIGGER policy_version_state_guard ON finance.policy_version;
CREATE TRIGGER policy_version_state_guard BEFORE UPDATE OF state OR DELETE ON finance.policy_version
 FOR EACH ROW EXECUTE FUNCTION finance.policy_version_state_guard();
ALTER TABLE finance.idempotency_record DROP CONSTRAINT dossier_change_receipt_required,
 ADD CONSTRAINT dossier_change_receipt_required CHECK(
 CASE WHEN operation_id IN ('dossier.revise','dossier.reopen','dossier.cancel','policy.create','policy.submit','policy.publish','policy.retire','policy.revoke','policy.revise')
 THEN result_payload IS NOT NULL AND jsonb_typeof(result_payload)='object'
 ELSE result_payload IS NULL END);
GRANT SELECT ON finance.document_ref,finance.upload_intent,finance.evidence,finance.ocr_run TO finance_policy_governance;
GRANT SELECT(tenant_id,company_id,id,sha256_digest,scan_state) ON finance.document_blob TO finance_policy_governance;
CREATE POLICY governance_document_ref_read ON finance.document_ref FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY governance_upload_intent_read ON finance.upload_intent FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY governance_evidence_read ON finance.evidence FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY governance_ocr_run_read ON finance.ocr_run FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY governance_document_blob_read ON finance.document_blob FOR SELECT TO finance_policy_governance
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE FUNCTION finance.runtime_policy_create(t uuid,c uuid,a uuid,pid uuid,idem text,pname text,kind text,starts date,ends date,source uuid,
 clause_keys text[],ev_ids uuid[],check_keys text[],doc_kinds text[],required_flags boolean[],cmd uuid,corr uuid)
RETURNS TABLE(id uuid,company_id uuid,version bigint,name text,state text,effective_from text,effective_to text,hash text)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE digest bytea; previous bytea; source_hash bytea; payload jsonb; affected bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR pid IS NULL OR source IS NULL OR cmd IS NULL OR corr IS NULL
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company() OR a IS DISTINCT FROM finance.scope_actor()
 OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$' OR pname IS NULL OR octet_length(pname) NOT BETWEEN 1 AND 200 OR btrim(pname)='' OR pname ~ '[[:cntrl:]]'
 OR kind IS NULL OR kind NOT IN ('PAYMENT','ADVANCE','SETTLEMENT') OR starts IS NULL OR ends IS NULL OR starts>=ends
 OR clause_keys IS NULL OR ev_ids IS NULL OR check_keys IS NULL OR doc_kinds IS NULL OR required_flags IS NULL
 OR cardinality(clause_keys) NOT BETWEEN 1 AND 200 OR cardinality(clause_keys)<>cardinality(ev_ids)
 OR cardinality(check_keys) NOT BETWEEN 1 AND 200 OR cardinality(check_keys)<>cardinality(doc_kinds) OR cardinality(check_keys)<>cardinality(required_flags)
 OR array_position(required_flags,NULL::boolean) IS NOT NULL OR NOT (true=ANY(required_flags))
 OR EXISTS(SELECT 1 FROM unnest(clause_keys) k WHERE k IS NULL OR k !~ '^[A-Za-z0-9._-]{1,64}$')
 OR EXISTS(SELECT 1 FROM unnest(doc_kinds) k WHERE k IS NULL OR k !~ '^[A-Z_]{1,64}$')
 OR EXISTS(SELECT 1 FROM unnest(check_keys) k WHERE k IS NULL OR NOT (k=ANY(clause_keys)))
 OR cardinality(clause_keys)<>(SELECT count(DISTINCT k) FROM unnest(clause_keys) k)
 OR cardinality(ev_ids)<>(SELECT count(DISTINCT e) FROM unnest(ev_ids) e)
 OR cardinality(doc_kinds)<>(SELECT count(DISTINCT k) FROM unnest(doc_kinds) k)
 THEN RAISE EXCEPTION 'invalid policy draft' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
  AND m.role='POLICY_ADMIN' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'policy maker required' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.policy.command/v1/'||t::text||'/'||c::text||'/'||a::text||'/policy.create/'||idem,0));
 SELECT b.sha256_digest INTO source_hash FROM finance.document_blob b
 JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id)=(b.tenant_id,b.company_id,b.id)
 JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(b.tenant_id,b.company_id,b.id)
 WHERE (b.tenant_id,b.company_id,b.id)=(t,c,source) AND b.scan_state='CLEAN' AND u.state='FINALIZED' AND u.actor_id=a;
 IF NOT FOUND THEN RAISE EXCEPTION 'clean authorized policy source required' USING ERRCODE='42501'; END IF;
 digest:=sha256(convert_to(jsonb_build_array(pname,kind,starts,ends,source,encode(source_hash,'hex'),clause_keys,ev_ids,check_keys,doc_kinds,required_flags)::text,'UTF8'));
 SELECT ir.request_hash,ir.result_payload INTO previous,payload FROM finance.idempotency_record ir
 WHERE (ir.tenant_id,ir.company_id,ir.actor_id,ir.operation_id,ir.key)=(t,c,a,'policy.create',idem);
 IF FOUND THEN
  IF previous<>digest THEN RAISE EXCEPTION 'policy replay conflict' USING ERRCODE='23514'; END IF;
 ELSE
  PERFORM finance.prototype_policy_lock(t,c,kind);
  INSERT INTO finance.policy_version(tenant_id,company_id,id,dossier_kind,author_id,effective_from,effective_until,source_digest,display_name)
   VALUES(t,c,pid,kind,a,starts,ends,source_hash,pname);
  INSERT INTO finance.policy_clause(tenant_id,company_id,policy_id,id,clause_key,excerpt,source_digest,source_page)
   SELECT t,c,pid,e.id,z.clause_key,e.excerpt,source_hash,e.page
   FROM unnest(clause_keys,ev_ids) z(clause_key,evidence_id)
   JOIN finance.evidence e ON (e.tenant_id,e.company_id,e.id)=(t,c,z.evidence_id)
   JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(e.tenant_id,e.company_id,e.ocr_run_id)
   WHERE o.document_id=source AND o.source_digest=source_hash;
  GET DIAGNOSTICS affected=ROW_COUNT;
  IF affected<>cardinality(clause_keys) THEN RAISE EXCEPTION 'policy evidence mismatch' USING ERRCODE='23514'; END IF;
  INSERT INTO finance.checklist_item(tenant_id,company_id,policy_id,id,clause_id,document_kind,mandatory)
   SELECT t,c,pid,gen_random_uuid(),cl.id,z.document_kind,z.mandatory
   FROM unnest(check_keys,doc_kinds,required_flags) z(clause_key,document_kind,mandatory)
   JOIN finance.policy_clause cl ON (cl.tenant_id,cl.company_id,cl.policy_id,cl.clause_key)=(t,c,pid,z.clause_key);
  GET DIAGNOSTICS affected=ROW_COUNT;
  IF affected<>cardinality(check_keys) THEN RAISE EXCEPTION 'checklist mapping mismatch' USING ERRCODE='23514'; END IF;
  payload:=jsonb_build_object('id',pid,'company_id',c,'version',1,'name',pname,'state','DRAFT',
   'effective_from',to_char(starts,'YYYY-MM-DD')||'T00:00:00Z','effective_to',to_char(ends,'YYYY-MM-DD')||'T00:00:00Z','hash',encode(source_hash,'hex'));
  INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
   VALUES(t,c,cmd,a,cmd,corr,pid,'POLICY_CREATED');
  INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES(t,c,cmd,cmd,pid,'POLICY_CREATED');
  INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_version,result_payload)
   VALUES(t,c,a,'policy.create',idem,digest,pid,1,payload);
 END IF;
 RETURN QUERY SELECT x.id,x.company_id,x.version,x.name,x.state,x.effective_from,x.effective_to,x.hash
 FROM jsonb_to_record(payload) AS x(id uuid,company_id uuid,version bigint,name text,state text,effective_from text,effective_to text,hash text);
END $$;
ALTER FUNCTION finance.runtime_policy_create(uuid,uuid,uuid,uuid,text,text,text,date,date,uuid,text[],uuid[],text[],text[],boolean[],uuid,uuid) OWNER TO finance_policy_governance;
REVOKE ALL ON FUNCTION finance.runtime_policy_create(uuid,uuid,uuid,uuid,text,text,text,date,date,uuid,text[],uuid[],text[],text[],boolean[],uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_policy_create(uuid,uuid,uuid,uuid,text,text,text,date,date,uuid,text[],uuid[],text[],text[],boolean[],uuid,uuid) TO finance_runtime;
ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','rule.revise','warning.ack','dossier.revise','dossier.reopen','dossier.cancel','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide','match.confirm','review.begin','review.decide','membership.create','membership.revoke',
 'risk.resolve','policy.create','policy.submit','policy.publish','policy.retire','policy.revoke','policy.revise','workflow.create','workflow.revise'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','WARNING_ACKNOWLEDGED','DOSSIER_REVISED','DOSSIER_REOPENED','DOSSIER_CANCELLED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','POLICY_CREATED','POLICY_SUBMITTED','POLICY_PUBLISHED','POLICY_RETIRED','POLICY_REVOKED','POLICY_REVISED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED','RULE_DRAFT_REVISED','WARNING_ACKNOWLEDGED','DOSSIER_REVISED','DOSSIER_REOPENED','DOSSIER_CANCELLED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED','RISK_RESOLVED','POLICY_CREATED','POLICY_SUBMITTED','POLICY_PUBLISHED','POLICY_RETIRED','POLICY_REVOKED','POLICY_REVISED','WORKFLOW_DRAFT_CREATED','WORKFLOW_DRAFT_REVISED'));
