-- Coordinated E09/E15/E18 synthetic fixture only. No live scan release,
-- no READY transition, no grant to runtime. Preserve immutable observations.
CREATE TABLE finance.review_assignment (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,dossier_id uuid NOT NULL,revision_id uuid NOT NULL,
 accountant_id uuid NOT NULL,assigned_by uuid NOT NULL,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,dossier_id,revision_id),
 FOREIGN KEY(tenant_id,company_id,dossier_id) REFERENCES finance.dossier(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,revision_id) REFERENCES finance.dossier_revision(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,accountant_id) REFERENCES finance.identity_subject(tenant_id,id),
 FOREIGN KEY(tenant_id,assigned_by) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(accountant_id<>assigned_by)
);
ALTER TABLE finance.review_assignment ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.review_assignment FORCE ROW LEVEL SECURITY;
CREATE TRIGGER review_assignment_immutable BEFORE UPDATE OR DELETE ON finance.review_assignment FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE FUNCTION finance.prototype_assign_candidate_review(t uuid,c uuid,d uuid,r uuid,a uuid,checker uuid)
RETURNS void LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $
DECLARE ds finance.dossier%ROWTYPE;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' OR a IS NULL OR checker IS NULL OR a=checker
 THEN RAISE EXCEPTION 'invalid reviewer assignment' USING ERRCODE='23514'; END IF;
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id IN (a,checker) OR ds.status<>'DRAFT'
 OR NOT EXISTS(SELECT 1 FROM finance.dossier_revision WHERE tenant_id=t AND company_id=c AND dossier_id=d AND id=r AND number=ds.revision)
 OR NOT EXISTS(SELECT 1 FROM finance.membership WHERE tenant_id=t AND company_id=c AND actor_id=a AND role='ACCOUNTANT' AND active AND valid_from<=clock_timestamp() AND (valid_until IS NULL OR valid_until>clock_timestamp()))
 OR NOT EXISTS(SELECT 1 FROM finance.membership WHERE tenant_id=t AND company_id=c AND actor_id=checker AND role='FINANCE_APPROVER' AND active AND valid_from<=clock_timestamp() AND (valid_until IS NULL OR valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'review assignment authority' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.review_assignment(tenant_id,company_id,dossier_id,revision_id,accountant_id,assigned_by) VALUES(t,c,d,r,a,checker);
END $;
REVOKE ALL ON FUNCTION finance.prototype_assign_candidate_review(uuid,uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
CREATE TABLE finance.field_decision (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,
 run_id uuid NOT NULL,field_path text NOT NULL,dossier_id uuid NOT NULL,revision_id uuid NOT NULL,
 dossier_version bigint NOT NULL CHECK(dossier_version BETWEEN 0 AND 9007199254740991),
 actor_id uuid NOT NULL,disposition text NOT NULL CHECK(disposition IN ('CONFIRMED_CANDIDATE','REJECTED')),
 normalized_value text,reason text NOT NULL CHECK(octet_length(reason) BETWEEN 1 AND 2048 AND btrim(reason)<>''),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),
 UNIQUE(tenant_id,company_id,run_id,field_path,dossier_version),
 FOREIGN KEY(tenant_id,company_id,run_id,field_path) REFERENCES finance.extracted_field(tenant_id,company_id,run_id,field_path),
 FOREIGN KEY(tenant_id,company_id,dossier_id) REFERENCES finance.dossier(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,revision_id) REFERENCES finance.dossier_revision(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK((disposition='CONFIRMED_CANDIDATE')=(normalized_value IS NOT NULL))
);
CREATE TABLE finance.check_run (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,
 dossier_id uuid NOT NULL,revision_id uuid NOT NULL,dossier_version bigint NOT NULL,
 actor_id uuid NOT NULL,policy_id uuid,business_day date NOT NULL,
 input_hash bytea NOT NULL CHECK(octet_length(input_hash)=32),
 blocked boolean NOT NULL CHECK(blocked),created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,dossier_id) REFERENCES finance.dossier(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,revision_id) REFERENCES finance.dossier_revision(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,policy_id) REFERENCES finance.policy_version(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id)
);
CREATE TABLE finance.check_result (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,run_id uuid NOT NULL,rule_key text NOT NULL,
 outcome text NOT NULL CHECK(outcome IN ('PASS','FAIL','UNKNOWN','ERROR','NOT_APPLICABLE')),
 severity text NOT NULL CHECK(severity IN ('BLOCK','WARNING','INFO')),
 detail_code text NOT NULL CHECK(detail_code ~ '^[A-Z_]{1,64}$'),
 PRIMARY KEY(tenant_id,company_id,run_id,rule_key),
 FOREIGN KEY(tenant_id,company_id,run_id) REFERENCES finance.check_run(tenant_id,company_id,id)
);
ALTER TABLE finance.field_decision ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.field_decision FORCE ROW LEVEL SECURITY;
CREATE TRIGGER field_decision_immutable BEFORE UPDATE OR DELETE ON finance.field_decision FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
ALTER TABLE finance.check_run ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.check_run FORCE ROW LEVEL SECURITY;
CREATE TRIGGER check_run_immutable BEFORE UPDATE OR DELETE ON finance.check_run FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
ALTER TABLE finance.check_result ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.check_result FORCE ROW LEVEL SECURITY;
CREATE TRIGGER check_result_immutable BEFORE UPDATE OR DELETE ON finance.check_result FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN ('ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN ('ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED','SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED'));
-- Fixture authority only: the trusted caller supplies actor UUID; no HTTP or live authentication.
CREATE FUNCTION finance.prototype_review_candidate(t uuid,c uuid,d uuid,v bigint,r uuid,path text,a uuid,i uuid,decision text,why text)
RETURNS uuid LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; candidate finance.extracted_field%ROWTYPE; current_run uuid; prior finance.field_decision%ROWTYPE;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' OR a IS NULL OR i IS NULL OR decision IS NULL
 OR decision NOT IN ('CONFIRMED_CANDIDATE','REJECTED') OR why IS NULL OR octet_length(why) NOT BETWEEN 1 AND 2048 OR btrim(why)=''
 THEN RAISE EXCEPTION 'invalid review command' USING ERRCODE='23514'; END IF;
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id=a OR ds.status<>'DRAFT'
 OR NOT EXISTS(SELECT 1 FROM finance.membership WHERE tenant_id=t AND company_id=c AND actor_id=a AND role='ACCOUNTANT' AND active
 AND valid_from<=clock_timestamp() AND (valid_until IS NULL OR valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'independent accountant required' USING ERRCODE='23514'; END IF;
 SELECT id INTO STRICT rev FROM finance.dossier_revision WHERE tenant_id=t AND company_id=c AND dossier_id=d AND number=ds.revision;
 IF NOT EXISTS(SELECT 1 FROM finance.review_assignment WHERE tenant_id=t AND company_id=c AND dossier_id=d AND revision_id=rev AND accountant_id=a)
 THEN RAISE EXCEPTION 'accountant not assigned' USING ERRCODE='23514'; END IF;
 SELECT * INTO prior FROM finance.field_decision WHERE tenant_id=t AND company_id=c AND id=i;
 IF FOUND THEN
  IF (prior.dossier_id,prior.run_id,prior.field_path,prior.actor_id,prior.disposition,prior.reason,prior.dossier_version)
   IS DISTINCT FROM (d,r,path,a,decision, v+1) OR prior.revision_id<>rev
  THEN RAISE EXCEPTION 'review replay conflict' USING ERRCODE='23505'; END IF;
  RETURN i;
 END IF;
 IF v IS NULL OR ds.version<>v OR v>=9007199254740991 THEN RAISE EXCEPTION 'stale draft' USING ERRCODE='23514'; END IF;
 SELECT f.* INTO candidate FROM finance.extracted_field f JOIN finance.extraction_run x ON (x.tenant_id,x.company_id,x.id)=(f.tenant_id,f.company_id,f.run_id)
 JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(x.tenant_id,x.company_id,x.ocr_run_id)
 JOIN finance.document_ref ref ON (ref.tenant_id,ref.company_id,ref.id)=(o.tenant_id,o.company_id,o.document_id)
 WHERE f.tenant_id=t AND f.company_id=c AND f.run_id=r AND f.field_path=path AND ref.dossier_id=d AND ref.revision_id=rev AND o.revision_id=rev;
 IF NOT FOUND THEN RAISE EXCEPTION 'wrong evidence binding' USING ERRCODE='23514'; END IF;
 SELECT x.id INTO current_run FROM finance.extraction_run x JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(x.tenant_id,x.company_id,x.ocr_run_id)
 WHERE x.tenant_id=t AND x.company_id=c AND o.revision_id=rev AND o.document_id=(SELECT document_id FROM finance.ocr_run WHERE tenant_id=t AND company_id=c AND id=(SELECT ocr_run_id FROM finance.extraction_run WHERE tenant_id=t AND company_id=c AND id=r))
 ORDER BY o.created_at DESC,o.id DESC LIMIT 1;
 IF current_run IS DISTINCT FROM r THEN RAISE EXCEPTION 'stale extraction' USING ERRCODE='23514'; END IF;
 IF decision='CONFIRMED_CANDIDATE' AND (candidate.state<>'CANDIDATE' OR cardinality(candidate.block_indexes)=0)
 THEN RAISE EXCEPTION 'candidate cannot be confirmed' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.field_decision(tenant_id,company_id,id,run_id,field_path,dossier_id,revision_id,dossier_version,actor_id,disposition,normalized_value,reason)
 VALUES(t,c,i,r,path,d,rev,v+1,a,decision,CASE WHEN decision='CONFIRMED_CANDIDATE' THEN candidate.normalized_value END,why);
 UPDATE finance.dossier SET version=version+1,updated_at=GREATEST(updated_at,clock_timestamp()) WHERE tenant_id=t AND company_id=c AND id=d;
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES(t,c,i,a,i,i,d,'SYNTHETIC_FIELD_REVIEWED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES(t,c,i,i,d,'SYNTHETIC_FIELD_REVIEWED');
 RETURN i;
END $$;
-- Read diagnostic inputs under draft and effective policy locks. The scanner and
-- full policy rules remain unapproved; unconditional BLOCK avoids fake readiness.
CREATE FUNCTION finance.prototype_preflight(t uuid,c uuid,d uuid,v bigint,a uuid,i uuid,day date)
RETURNS uuid LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; p uuid; digest bytea; docs jsonb; reviews jsonb; observations jsonb; prior finance.check_run%ROWTYPE; item record; present boolean;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' OR day IS NULL OR i IS NULL OR v IS NULL
 THEN RAISE EXCEPTION 'invalid preflight command' USING ERRCODE='23514'; END IF;
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id IS DISTINCT FROM a OR ds.status<>'DRAFT'
 OR NOT EXISTS(SELECT 1 FROM finance.membership WHERE tenant_id=t AND company_id=c AND actor_id=a AND role='REQUESTER' AND active AND valid_from<=clock_timestamp() AND (valid_until IS NULL OR valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'requester required' USING ERRCODE='23514'; END IF;
 SELECT id INTO STRICT rev FROM finance.dossier_revision WHERE tenant_id=t AND company_id=c AND dossier_id=d AND number=ds.revision;
 SELECT * INTO prior FROM finance.check_run WHERE tenant_id=t AND company_id=c AND id=i;
 IF FOUND THEN
  IF (prior.dossier_id,prior.dossier_version,prior.actor_id,prior.business_day,prior.revision_id) IS DISTINCT FROM (d,v,a,day,rev)
  THEN RAISE EXCEPTION 'preflight replay conflict' USING ERRCODE='23505'; END IF;
  RETURN i;
 END IF;
 IF ds.version<>v THEN RAISE EXCEPTION 'stale preflight' USING ERRCODE='23514'; END IF;
 PERFORM finance.prototype_policy_lock(t,c,ds.kind);
 p:=finance.prototype_effective_policy(t,c,ds.kind,day);
 SELECT COALESCE(jsonb_agg(jsonb_build_array(u.id,u.generation,u.state,u.document_kind,encode(u.declared_sha256,'hex'),b.scan_state) ORDER BY u.id),'[]'::jsonb) INTO docs
 FROM finance.upload_intent u LEFT JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(u.tenant_id,u.company_id,u.id)
 WHERE u.tenant_id=t AND u.company_id=c AND u.dossier_id=d AND u.revision_id=rev AND u.state<>'CANCELLED';
 SELECT COALESCE(jsonb_agg(jsonb_build_array(id,run_id,field_path,disposition,normalized_value,dossier_version) ORDER BY dossier_version,id),'[]'::jsonb) INTO reviews
 FROM finance.field_decision WHERE tenant_id=t AND company_id=c AND dossier_id=d AND revision_id=rev;
 SELECT COALESCE(jsonb_agg(jsonb_build_array(o.id,o.document_id,encode(o.result_digest,'hex'),o.generation,o.fence) ORDER BY o.id),'[]'::jsonb) INTO observations
 FROM finance.ocr_run o JOIN finance.document_ref f ON (f.tenant_id,f.company_id,f.id)=(o.tenant_id,o.company_id,o.document_id)
 WHERE o.tenant_id=t AND o.company_id=c AND o.revision_id=rev AND f.dossier_id=d AND f.revision_id=rev;
 digest:=sha256(convert_to(jsonb_build_array(d,rev,v,p,day,docs,reviews,observations)::text,'UTF8'));
 INSERT INTO finance.check_run(tenant_id,company_id,id,dossier_id,revision_id,dossier_version,actor_id,policy_id,business_day,input_hash,blocked)
 VALUES(t,c,i,d,rev,v,a,p,day,digest,true);
 INSERT INTO finance.check_result VALUES(t,c,i,'policy.effective',CASE WHEN p IS NULL THEN 'UNKNOWN' ELSE 'PASS' END,'BLOCK',CASE WHEN p IS NULL THEN 'MISSING_POLICY' ELSE 'SYNTHETIC_POLICY_FOUND' END);
 IF p IS NOT NULL THEN
  FOR item IN SELECT document_kind,mandatory FROM finance.checklist_item WHERE tenant_id=t AND company_id=c AND policy_id=p ORDER BY document_kind LOOP
   SELECT EXISTS(SELECT 1 FROM finance.upload_intent WHERE tenant_id=t AND company_id=c AND dossier_id=d AND revision_id=rev AND document_kind=item.document_kind AND state<>'CANCELLED') INTO present;
   INSERT INTO finance.check_result VALUES(t,c,i,'checklist.'||item.document_kind,CASE WHEN present THEN 'UNKNOWN' ELSE 'FAIL' END,CASE WHEN item.mandatory THEN 'BLOCK' ELSE 'WARNING' END,CASE WHEN present THEN 'DOCUMENT_NOT_RELEASED' ELSE 'MISSING_DOCUMENT' END);
  END LOOP;
 END IF;
 INSERT INTO finance.check_result VALUES
 (t,c,i,'scan.release','UNKNOWN','BLOCK','NO_APPROVED_SCAN_RELEASE'),
 (t,c,i,'rules.complete','UNKNOWN','BLOCK','NO_APPROVED_RULE_SET'),
 (t,c,i,'financial.reservation','UNKNOWN','BLOCK','NO_BOUND_RESERVATION'),
 (t,c,i,'critical.fields','UNKNOWN','BLOCK','NO_APPROVED_CRITICAL_REGISTRY');
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES(t,c,i,a,i,i,d,'SYNTHETIC_PREFLIGHT_CHECKED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES(t,c,i,i,d,'SYNTHETIC_PREFLIGHT_CHECKED');
 RETURN i;
END $$;
REVOKE ALL ON FUNCTION finance.prototype_review_candidate(uuid,uuid,uuid,bigint,uuid,text,uuid,uuid,text,text),
 finance.prototype_preflight(uuid,uuid,uuid,bigint,uuid,uuid,date) FROM PUBLIC;
