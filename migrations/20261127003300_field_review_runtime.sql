-- E08/E09 runtime field projection and independent accountant decisions over persisted OCR evidence.
CREATE ROLE finance_field_review_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_field_review_writer;

CREATE TABLE finance.field_identity (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,run_id uuid NOT NULL,field_path text NOT NULL,id uuid NOT NULL,
 PRIMARY KEY(tenant_id,company_id,run_id,field_path),UNIQUE(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,run_id,field_path)
  REFERENCES finance.extracted_field(tenant_id,company_id,run_id,field_path)
);
INSERT INTO finance.field_identity(tenant_id,company_id,run_id,field_path,id)
 SELECT tenant_id,company_id,run_id,field_path,gen_random_uuid() FROM finance.extracted_field;
ALTER TABLE finance.field_identity ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.field_identity FORCE ROW LEVEL SECURITY;

ALTER TABLE finance.field_decision DROP CONSTRAINT field_decision_disposition_check;
ALTER TABLE finance.field_decision DROP CONSTRAINT field_decision_check;
ALTER TABLE finance.field_decision
 ADD COLUMN evidence_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[],
 ADD CONSTRAINT field_decision_disposition_check
  CHECK(disposition IN ('CONFIRMED_CANDIDATE','CORRECTED','REJECTED')),
 ADD CONSTRAINT field_decision_value_check
  CHECK((disposition IN ('CONFIRMED_CANDIDATE','CORRECTED'))=(normalized_value IS NOT NULL)),
 ADD CONSTRAINT field_decision_evidence_check CHECK(cardinality(evidence_ids)<=64);

GRANT SELECT ON finance.dossier,finance.dossier_revision,finance.membership,finance.review_assignment,
 finance.document_ref,finance.ocr_run,finance.evidence,finance.extraction_run,finance.extracted_field,
 finance.field_identity,finance.field_decision TO finance_field_review_writer;
GRANT INSERT ON finance.field_identity,finance.field_decision,finance.audit_event,finance.outbox
 TO finance_field_review_writer;
GRANT UPDATE(version,updated_at) ON finance.dossier TO finance_field_review_writer;

CREATE POLICY field_writer_dossier_read ON finance.dossier FOR SELECT TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_dossier_update ON finance.dossier FOR UPDATE TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_revision_read ON finance.dossier_revision FOR SELECT TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_membership_read ON finance.membership FOR SELECT TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_assignment_read ON finance.review_assignment FOR SELECT TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_document_read ON finance.document_ref FOR SELECT TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_ocr_read ON finance.ocr_run FOR SELECT TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_evidence_read ON finance.evidence FOR SELECT TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_extraction_read ON finance.extraction_run FOR SELECT TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_extracted_read ON finance.extracted_field FOR SELECT TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_identity_read ON finance.field_identity FOR SELECT TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_identity_insert ON finance.field_identity FOR INSERT TO finance_field_review_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_decision_read ON finance.field_decision FOR SELECT TO finance_field_review_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_decision_insert ON finance.field_decision FOR INSERT TO finance_field_review_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_audit_insert ON finance.audit_event FOR INSERT TO finance_field_review_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY field_writer_outbox_insert ON finance.outbox FOR INSERT TO finance_field_review_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());

CREATE FUNCTION finance.field_identity_after_insert() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $
BEGIN
 INSERT INTO finance.field_identity(tenant_id,company_id,run_id,field_path,id)
 VALUES(NEW.tenant_id,NEW.company_id,NEW.run_id,NEW.field_path,gen_random_uuid());
 RETURN NEW;
END $;
ALTER FUNCTION finance.field_identity_after_insert() OWNER TO finance_field_review_writer;
REVOKE ALL ON FUNCTION finance.field_identity_after_insert() FROM PUBLIC;
CREATE TRIGGER field_identity_after_extract AFTER INSERT ON finance.extracted_field
 FOR EACH ROW EXECUTE FUNCTION finance.field_identity_after_insert();

CREATE FUNCTION finance.field_access_allowed(t uuid,c uuid,d uuid,a uuid)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT t=finance.scope_tenant() AND c=finance.scope_company() AND a=finance.scope_actor()
 AND EXISTS(
  SELECT 1 FROM finance.dossier ds
  JOIN finance.dossier_revision dr
    ON (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number)=(ds.tenant_id,ds.company_id,ds.id,ds.revision)
  WHERE ds.tenant_id=t AND ds.company_id=c AND ds.id=d
  AND (
   (ds.owner_id=a AND EXISTS(
    SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c AND m.actor_id=a
     AND m.role='REQUESTER' AND m.active AND m.valid_from<=statement_timestamp()
     AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())
   ))
   OR
   (EXISTS(
    SELECT 1 FROM finance.review_assignment ra
    WHERE ra.tenant_id=t AND ra.company_id=c AND ra.dossier_id=d AND ra.revision_id=dr.id AND ra.accountant_id=a
   ) AND EXISTS(
    SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c AND m.actor_id=a
     AND m.role='ACCOUNTANT' AND m.active AND m.valid_from<=statement_timestamp()
     AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())
   ))
  )
 )
$$;
ALTER FUNCTION finance.field_access_allowed(uuid,uuid,uuid,uuid) OWNER TO finance_field_review_writer;
REVOKE ALL ON FUNCTION finance.field_access_allowed(uuid,uuid,uuid,uuid) FROM PUBLIC;

CREATE FUNCTION finance.runtime_fields(t uuid,c uuid,d uuid,a uuid,after_id uuid,only_id uuid,n integer)
RETURNS TABLE(
 id uuid,path text,raw_value text,normalized_value text,value_type text,state text,
 evidence_ids uuid[],extractor_version text
)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF n IS NULL OR n NOT BETWEEN 1 AND 100 OR after_id IS NULL
 OR NOT finance.field_access_allowed(t,c,d,a)
 THEN RAISE EXCEPTION 'field read forbidden or invalid' USING ERRCODE='42501'; END IF;
 RETURN QUERY
 WITH current_rows AS (
  SELECT fi.id,f.run_id,f.field_path,f.state AS source_state,f.normalized_value AS source_value,
   f.source_page,f.block_indexes,x.schema_version,o.id AS ocr_id,
   ds.version AS dossier_version,fd.disposition,fd.normalized_value AS decision_value,
   fd.evidence_ids AS decision_evidence
  FROM finance.dossier ds
  JOIN finance.dossier_revision dr
    ON (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number)=(ds.tenant_id,ds.company_id,ds.id,ds.revision)
  JOIN finance.document_ref r
    ON (r.tenant_id,r.company_id,r.dossier_id,r.revision_id)=(ds.tenant_id,ds.company_id,ds.id,dr.id)
  JOIN finance.ocr_run o
    ON (o.tenant_id,o.company_id,o.document_id,o.revision_id)=(r.tenant_id,r.company_id,r.id,dr.id)
  JOIN finance.extraction_run x
    ON (x.tenant_id,x.company_id,x.ocr_run_id)=(o.tenant_id,o.company_id,o.id)
  JOIN finance.extracted_field f
    ON (f.tenant_id,f.company_id,f.run_id)=(x.tenant_id,x.company_id,x.id)
  JOIN finance.field_identity fi
    ON (fi.tenant_id,fi.company_id,fi.run_id,fi.field_path)=(f.tenant_id,f.company_id,f.run_id,f.field_path)
  LEFT JOIN LATERAL (
   SELECT q.disposition,q.normalized_value,q.evidence_ids
   FROM finance.field_decision q
   WHERE q.tenant_id=f.tenant_id AND q.company_id=f.company_id AND q.run_id=f.run_id
    AND q.field_path=f.field_path AND q.dossier_id=ds.id AND q.revision_id=dr.id
   ORDER BY q.dossier_version DESC,q.created_at DESC,q.id DESC LIMIT 1
  ) fd ON true
  WHERE ds.tenant_id=t AND ds.company_id=c AND ds.id=d
   AND (only_id IS NULL OR fi.id=only_id)
   AND (only_id IS NOT NULL OR fi.id>after_id)
   AND NOT EXISTS(
    SELECT 1 FROM finance.ocr_run newer
    WHERE newer.tenant_id=o.tenant_id AND newer.company_id=o.company_id
     AND newer.document_id=o.document_id AND newer.revision_id=o.revision_id
     AND (newer.created_at>o.created_at OR (newer.created_at=o.created_at AND newer.id>o.id))
   )
  ORDER BY fi.id
  LIMIT n
 )
 SELECT row.id,row.field_path,
  (SELECT string_agg(e.excerpt,' | ' ORDER BY e.block_index,e.id)
   FROM finance.evidence e
   WHERE e.tenant_id=t AND e.company_id=c AND e.ocr_run_id=row.ocr_id
    AND e.page=row.source_page AND e.block_index=ANY(row.block_indexes)) AS raw_value,
  CASE
   WHEN row.disposition IN ('CONFIRMED_CANDIDATE','CORRECTED') THEN row.decision_value
   WHEN row.disposition='REJECTED' THEN NULL
   WHEN row.source_state='CANDIDATE' THEN row.source_value
   ELSE NULL END AS normalized_value,
  CASE row.field_path WHEN 'invoice.date' THEN 'date'
   WHEN 'invoice.net' THEN 'money' WHEN 'invoice.tax' THEN 'money' WHEN 'invoice.total' THEN 'money'
   ELSE 'text' END AS value_type,
  CASE row.disposition WHEN 'CONFIRMED_CANDIDATE' THEN 'VERIFIED'
   WHEN 'CORRECTED' THEN 'CORRECTED' WHEN 'REJECTED' THEN 'UNKNOWN'
   ELSE CASE WHEN row.source_state='CANDIDATE' THEN 'CANDIDATE' ELSE 'UNKNOWN' END END AS state,
  CASE WHEN row.disposition IS NOT NULL AND cardinality(row.decision_evidence)>0
   THEN row.decision_evidence
   ELSE COALESCE((SELECT array_agg(e.id ORDER BY e.block_index,e.id)
    FROM finance.evidence e
    WHERE e.tenant_id=t AND e.company_id=c AND e.ocr_run_id=row.ocr_id
     AND e.page=row.source_page AND e.block_index=ANY(row.block_indexes)),ARRAY[]::uuid[]) END AS evidence_ids,
  row.schema_version AS extractor_version
 FROM current_rows row
 ORDER BY row.id;
END $$;
ALTER FUNCTION finance.runtime_fields(uuid,uuid,uuid,uuid,uuid,uuid,integer) OWNER TO finance_field_review_writer;
REVOKE ALL ON FUNCTION finance.runtime_fields(uuid,uuid,uuid,uuid,uuid,uuid,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_fields(uuid,uuid,uuid,uuid,uuid,uuid,integer) TO finance_runtime;

CREATE FUNCTION finance.runtime_field_meta(t uuid,c uuid,d uuid,a uuid,field_id uuid)
RETURNS TABLE(path text,value_type text,current_value text)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF field_id IS NULL OR NOT finance.field_access_allowed(t,c,d,a)
 THEN RAISE EXCEPTION 'field metadata forbidden' USING ERRCODE='42501'; END IF;
 RETURN QUERY
 SELECT f.field_path,
  CASE f.field_path WHEN 'invoice.date' THEN 'date'
   WHEN 'invoice.net' THEN 'money' WHEN 'invoice.tax' THEN 'money' WHEN 'invoice.total' THEN 'money'
   ELSE 'text' END,
  f.normalized_value
 FROM finance.field_identity fi
 JOIN finance.extracted_field f
  ON (f.tenant_id,f.company_id,f.run_id,f.field_path)=(fi.tenant_id,fi.company_id,fi.run_id,fi.field_path)
 JOIN finance.extraction_run x
  ON (x.tenant_id,x.company_id,x.id)=(f.tenant_id,f.company_id,f.run_id)
 JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(x.tenant_id,x.company_id,x.ocr_run_id)
 JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(o.tenant_id,o.company_id,o.document_id)
 JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(r.tenant_id,r.company_id,r.dossier_id)
 JOIN finance.dossier_revision dr
  ON (dr.tenant_id,dr.company_id,dr.id,dr.number)=(ds.tenant_id,ds.company_id,r.revision_id,ds.revision)
 WHERE fi.tenant_id=t AND fi.company_id=c AND fi.id=field_id AND ds.id=d
  AND o.revision_id=dr.id
  AND NOT EXISTS(SELECT 1 FROM finance.ocr_run newer
   WHERE newer.tenant_id=o.tenant_id AND newer.company_id=o.company_id
    AND newer.document_id=o.document_id AND newer.revision_id=o.revision_id
    AND (newer.created_at>o.created_at OR (newer.created_at=o.created_at AND newer.id>o.id)))
 LIMIT 1;
END $$;
ALTER FUNCTION finance.runtime_field_meta(uuid,uuid,uuid,uuid,uuid) OWNER TO finance_field_review_writer;
REVOKE ALL ON FUNCTION finance.runtime_field_meta(uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_field_meta(uuid,uuid,uuid,uuid,uuid) TO finance_runtime;

CREATE FUNCTION finance.runtime_field_decide(
 t uuid,c uuid,d uuid,v bigint,field_id uuid,a uuid,decision_id uuid,new_value text,
 reason text,decision_evidence uuid[],cmd uuid,corr uuid
) RETURNS bigint
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; rev uuid; f finance.extracted_field%ROWTYPE; o finance.ocr_run%ROWTYPE;
 source_evidence uuid[]; evidence_count bigint; disposition text; new_version bigint;
BEGIN
 IF t IS NULL OR c IS NULL OR d IS NULL OR v IS NULL OR v<0 OR field_id IS NULL OR a IS NULL
 OR decision_id IS NULL OR cmd IS NULL OR corr IS NULL OR new_value IS NULL
 OR octet_length(new_value) NOT BETWEEN 1 AND 512 OR new_value<>btrim(new_value)
 OR reason IS NULL OR octet_length(reason) NOT BETWEEN 1 AND 2048 OR btrim(reason)=''
 OR decision_evidence IS NULL OR cardinality(decision_evidence)>64
 OR t IS DISTINCT FROM finance.scope_tenant() OR c IS DISTINCT FROM finance.scope_company()
 OR a IS DISTINCT FROM finance.scope_actor()
 THEN RAISE EXCEPTION 'invalid field decision' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO ds FROM finance.dossier WHERE tenant_id=t AND company_id=c AND id=d FOR UPDATE;
 IF NOT FOUND OR ds.owner_id=a OR ds.status NOT IN ('DRAFT','NEEDS_INPUT','REVIEWABLE') OR ds.version<>v
 THEN RAISE EXCEPTION 'field decision state or self review' USING ERRCODE='23514'; END IF;
 SELECT id INTO STRICT rev FROM finance.dossier_revision
 WHERE tenant_id=t AND company_id=c AND dossier_id=d AND number=ds.revision;
 IF NOT EXISTS(SELECT 1 FROM finance.review_assignment ra WHERE ra.tenant_id=t AND ra.company_id=c
  AND ra.dossier_id=d AND ra.revision_id=rev AND ra.accountant_id=a)
 OR NOT EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c
  AND m.actor_id=a AND m.role='ACCOUNTANT' AND m.active AND m.valid_from<=clock_timestamp()
  AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'assigned accountant required' USING ERRCODE='23514'; END IF;
 SELECT ef.* INTO f
 FROM finance.field_identity fi
 JOIN finance.extracted_field ef
  ON (ef.tenant_id,ef.company_id,ef.run_id,ef.field_path)=(fi.tenant_id,fi.company_id,fi.run_id,fi.field_path)
 JOIN finance.extraction_run x
  ON (x.tenant_id,x.company_id,x.id)=(ef.tenant_id,ef.company_id,ef.run_id)
 JOIN finance.ocr_run ocr ON (ocr.tenant_id,ocr.company_id,ocr.id)=(x.tenant_id,x.company_id,x.ocr_run_id)
 JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(ocr.tenant_id,ocr.company_id,ocr.document_id)
 WHERE fi.tenant_id=t AND fi.company_id=c AND fi.id=field_id AND r.dossier_id=d AND r.revision_id=rev
  AND ocr.revision_id=rev
  AND NOT EXISTS(SELECT 1 FROM finance.ocr_run newer
   WHERE newer.tenant_id=ocr.tenant_id AND newer.company_id=ocr.company_id
    AND newer.document_id=ocr.document_id AND newer.revision_id=ocr.revision_id
    AND (newer.created_at>ocr.created_at OR (newer.created_at=ocr.created_at AND newer.id>ocr.id)));
 IF NOT FOUND THEN RAISE EXCEPTION 'stale or unknown field' USING ERRCODE='23514'; END IF;
 SELECT ocr.* INTO STRICT o
 FROM finance.extraction_run x
 JOIN finance.ocr_run ocr
  ON (ocr.tenant_id,ocr.company_id,ocr.id)=(x.tenant_id,x.company_id,x.ocr_run_id)
 WHERE x.tenant_id=t AND x.company_id=c AND x.id=f.run_id;
 IF EXISTS(SELECT 1 FROM finance.field_decision q WHERE q.tenant_id=t AND q.company_id=c
  AND q.run_id=f.run_id AND q.field_path=f.field_path AND q.dossier_id=d AND q.revision_id=rev)
 THEN RAISE EXCEPTION 'current extraction field already decided' USING ERRCODE='23514'; END IF;
 SELECT COALESCE(array_agg(e.id ORDER BY e.block_index,e.id),ARRAY[]::uuid[]) INTO source_evidence
 FROM finance.evidence e WHERE e.tenant_id=t AND e.company_id=c AND e.ocr_run_id=o.id
  AND e.page=f.source_page AND e.block_index=ANY(f.block_indexes);
 IF f.state='CANDIDATE' AND f.normalized_value=new_value
  AND cardinality(decision_evidence)=cardinality(source_evidence)
  AND decision_evidence @> source_evidence AND decision_evidence <@ source_evidence
 THEN disposition:='CONFIRMED_CANDIDATE';
 ELSE
  disposition:='CORRECTED';
  IF cardinality(decision_evidence)=0 THEN
   RAISE EXCEPTION 'correction evidence required' USING ERRCODE='23514'; END IF;
  SELECT count(DISTINCT e.id) INTO evidence_count
  FROM finance.evidence e JOIN finance.ocr_run eo
   ON (eo.tenant_id,eo.company_id,eo.id)=(e.tenant_id,e.company_id,e.ocr_run_id)
  JOIN finance.document_ref r
   ON (r.tenant_id,r.company_id,r.id)=(eo.tenant_id,eo.company_id,eo.document_id)
  WHERE e.tenant_id=t AND e.company_id=c AND e.id=ANY(decision_evidence)
   AND r.dossier_id=d AND r.revision_id=rev AND eo.revision_id=rev;
  IF evidence_count<>cardinality(decision_evidence)
   OR cardinality(decision_evidence)<>(SELECT count(DISTINCT x) FROM unnest(decision_evidence) x)
  THEN RAISE EXCEPTION 'correction evidence binding' USING ERRCODE='23514'; END IF;
 END IF;
 INSERT INTO finance.field_decision(
  tenant_id,company_id,id,run_id,field_path,dossier_id,revision_id,dossier_version,
  actor_id,disposition,normalized_value,reason,evidence_ids
 ) VALUES(t,c,decision_id,f.run_id,f.field_path,d,rev,v+1,a,disposition,new_value,reason,decision_evidence);
 UPDATE finance.dossier SET version=version+1,updated_at=GREATEST(updated_at,clock_timestamp())
 WHERE tenant_id=t AND company_id=c AND id=d RETURNING version INTO new_version;
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
 VALUES(t,c,cmd,a,cmd,corr,d,'FIELD_REVIEWED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
 VALUES(t,c,cmd,cmd,d,'FIELD_REVIEWED');
 RETURN new_version;
END $$;
ALTER FUNCTION finance.runtime_field_decide(uuid,uuid,uuid,bigint,uuid,uuid,uuid,text,text,uuid[],uuid,uuid)
 OWNER TO finance_field_review_writer;
REVOKE ALL ON FUNCTION finance.runtime_field_decide(uuid,uuid,uuid,bigint,uuid,uuid,uuid,text,text,uuid[],uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_field_decide(uuid,uuid,uuid,bigint,uuid,uuid,uuid,text,text,uuid[],uuid,uuid)
 TO finance_runtime;

CREATE FUNCTION finance.runtime_field_receipt(t uuid,c uuid,d uuid,a uuid,decision_id uuid)
RETURNS TABLE(
 id uuid,path text,raw_value text,normalized_value text,value_type text,state text,
 evidence_ids uuid[],extractor_version text,dossier_version bigint
)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF decision_id IS NULL OR NOT finance.field_access_allowed(t,c,d,a)
 THEN RAISE EXCEPTION 'field receipt forbidden' USING ERRCODE='42501'; END IF;
 RETURN QUERY
 SELECT fi.id,f.field_path,
  (SELECT string_agg(e.excerpt,' | ' ORDER BY e.block_index,e.id)
   FROM finance.evidence e JOIN finance.extraction_run x2
    ON (x2.tenant_id,x2.company_id,x2.ocr_run_id)=(e.tenant_id,e.company_id,e.ocr_run_id)
   WHERE x2.tenant_id=f.tenant_id AND x2.company_id=f.company_id AND x2.id=f.run_id
    AND e.page=f.source_page AND e.block_index=ANY(f.block_indexes)),
  q.normalized_value,
  CASE f.field_path WHEN 'invoice.date' THEN 'date'
   WHEN 'invoice.net' THEN 'money' WHEN 'invoice.tax' THEN 'money' WHEN 'invoice.total' THEN 'money'
   ELSE 'text' END,
  CASE q.disposition WHEN 'CONFIRMED_CANDIDATE' THEN 'VERIFIED'
   WHEN 'CORRECTED' THEN 'CORRECTED' ELSE 'UNKNOWN' END,
  q.evidence_ids,x.schema_version,q.dossier_version
 FROM finance.field_decision q
 JOIN finance.field_identity fi
  ON (fi.tenant_id,fi.company_id,fi.run_id,fi.field_path)=(q.tenant_id,q.company_id,q.run_id,q.field_path)
 JOIN finance.extracted_field f
  ON (f.tenant_id,f.company_id,f.run_id,f.field_path)=(fi.tenant_id,fi.company_id,fi.run_id,fi.field_path)
 JOIN finance.extraction_run x
  ON (x.tenant_id,x.company_id,x.id)=(f.tenant_id,f.company_id,f.run_id)
 WHERE q.tenant_id=t AND q.company_id=c AND q.dossier_id=d AND q.actor_id=a AND q.id=decision_id
 LIMIT 1;
END $$;
ALTER FUNCTION finance.runtime_field_receipt(uuid,uuid,uuid,uuid,uuid) OWNER TO finance_field_review_writer;
REVOKE ALL ON FUNCTION finance.runtime_field_receipt(uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_field_receipt(uuid,uuid,uuid,uuid,uuid) TO finance_runtime;

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide'));
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED'));
