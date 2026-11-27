-- E05: scoped current-revision OCR evidence projection; never expose raw bytes.
CREATE FUNCTION finance.runtime_evidence(t uuid,c uuid,a uuid,e uuid)
RETURNS TABLE(id uuid,document_id uuid,sha256 text,page integer,locator text,excerpt text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT ev.id,o.document_id,encode(o.source_digest,'hex'),ev.page,
  format('rect:%s,%s,%s,%s',ev.x0,ev.y0,ev.x1,ev.y1),ev.excerpt
 FROM finance.evidence ev
 JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(ev.tenant_id,ev.company_id,ev.ocr_run_id)
 JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id,r.revision_id)=
  (o.tenant_id,o.company_id,o.document_id,o.revision_id)
 JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(r.tenant_id,r.company_id,r.dossier_id)
 JOIN finance.dossier_revision dr ON (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number,dr.id)=
  (ds.tenant_id,ds.company_id,ds.id,ds.revision,r.revision_id)
 WHERE (ev.tenant_id,ev.company_id,ev.id)=(t,c,e)
  AND finance.field_access_allowed(t,c,ds.id,a)
$$;
ALTER FUNCTION finance.runtime_evidence(uuid,uuid,uuid,uuid) OWNER TO finance_field_review_writer;
REVOKE ALL ON FUNCTION finance.runtime_evidence(uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_evidence(uuid,uuid,uuid,uuid) TO finance_runtime;
