-- E05 metadata projection: runtime can read scan disposition without raw document bytes.
CREATE ROLE finance_document_metadata_reader NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_document_metadata_reader;
GRANT SELECT ON finance.upload_intent,finance.document_blob TO finance_document_metadata_reader;

CREATE POLICY document_metadata_reader_intent ON finance.upload_intent
 FOR SELECT TO finance_document_metadata_reader USING(
  tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
  AND actor_id=finance.scope_actor()
);
CREATE POLICY document_metadata_reader_blob ON finance.document_blob
 FOR SELECT TO finance_document_metadata_reader USING(
  tenant_id=finance.scope_tenant() AND company_id=finance.scope_company()
  AND EXISTS(
   SELECT 1 FROM finance.upload_intent u
   WHERE (u.tenant_id,u.company_id,u.id)=(document_blob.tenant_id,document_blob.company_id,document_blob.id)
   AND u.actor_id=finance.scope_actor()
  )
);

CREATE FUNCTION finance.runtime_document_state(t uuid,c uuid,a uuid,i uuid)
RETURNS text LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT CASE u.state
  WHEN 'FINALIZED' THEN COALESCE(b.scan_state,'QUARANTINED')
  ELSE u.state
 END
 FROM finance.upload_intent u
 LEFT JOIN finance.document_blob b
  ON (b.tenant_id,b.company_id,b.id)=(u.tenant_id,u.company_id,u.id)
 WHERE t=finance.scope_tenant() AND c=finance.scope_company() AND a=finance.scope_actor()
 AND (u.tenant_id,u.company_id,u.actor_id,u.id)=(t,c,a,i)
$$;
ALTER FUNCTION finance.runtime_document_state(uuid,uuid,uuid,uuid) OWNER TO finance_document_metadata_reader;
REVOKE ALL ON FUNCTION finance.runtime_document_state(uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_document_state(uuid,uuid,uuid,uuid) TO finance_runtime;
