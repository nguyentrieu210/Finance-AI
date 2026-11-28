-- E15 checklist projection pinned to the current immutable check run.
CREATE ROLE finance_checklist_reader NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_checklist_reader;
GRANT SELECT ON finance.dossier,finance.dossier_revision,finance.check_run,
 finance.checklist_item,finance.upload_intent,finance.document_blob,finance.membership
 TO finance_checklist_reader;
CREATE POLICY checklist_reader_dossier ON finance.dossier FOR SELECT TO finance_checklist_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY checklist_reader_revision ON finance.dossier_revision FOR SELECT TO finance_checklist_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY checklist_reader_run ON finance.check_run FOR SELECT TO finance_checklist_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY checklist_reader_item ON finance.checklist_item FOR SELECT TO finance_checklist_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY checklist_reader_intent ON finance.upload_intent FOR SELECT TO finance_checklist_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY checklist_reader_blob ON finance.document_blob FOR SELECT TO finance_checklist_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY checklist_reader_membership ON finance.membership FOR SELECT TO finance_checklist_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());

CREATE FUNCTION finance.runtime_checklist(
 t uuid,c uuid,a uuid,d uuid,after_id uuid,n integer
) RETURNS TABLE(id uuid,requirement text,state text,clause_id uuid,evidence_ids uuid[])
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 WITH target AS (
  SELECT ds.id,dr.id AS revision_id,ds.version
  FROM finance.dossier ds
  JOIN finance.dossier_revision dr
   ON (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number)=
      (ds.tenant_id,ds.company_id,ds.id,ds.revision)
  WHERE (ds.tenant_id,ds.company_id,ds.id,ds.owner_id)=(t,c,d,a)
   AND EXISTS(
    SELECT 1 FROM finance.membership m
    WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
     AND m.role='REQUESTER' AND m.active
     AND m.valid_from<=statement_timestamp()
     AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())
   )
 ), run AS (
  SELECT r.policy_id
  FROM finance.check_run r,target x
  WHERE (r.tenant_id,r.company_id,r.dossier_id,r.revision_id,r.dossier_version,r.actor_id)=
        (t,c,d,x.revision_id,x.version,a)
   AND r.policy_id IS NOT NULL
  ORDER BY r.created_at DESC,r.id DESC
  LIMIT 1
 )
 SELECT ci.id,ci.document_kind,
  CASE
   WHEN EXISTS(
    SELECT 1 FROM finance.upload_intent u
    JOIN target x ON x.revision_id=u.revision_id
    JOIN finance.document_blob b
     ON (b.tenant_id,b.company_id,b.id)=(u.tenant_id,u.company_id,u.id)
    WHERE (u.tenant_id,u.company_id,u.dossier_id,u.actor_id,u.document_kind)=
          (t,c,d,a,ci.document_kind)
     AND u.state='FINALIZED' AND b.scan_state='CLEAN'
   ) THEN 'PRESENT'
   WHEN EXISTS(
    SELECT 1 FROM finance.upload_intent u
    JOIN target x ON x.revision_id=u.revision_id
    JOIN finance.document_blob b
     ON (b.tenant_id,b.company_id,b.id)=(u.tenant_id,u.company_id,u.id)
    WHERE (u.tenant_id,u.company_id,u.dossier_id,u.actor_id,u.document_kind)=
          (t,c,d,a,ci.document_kind)
     AND u.state='FINALIZED' AND b.scan_state='REJECTED'
   ) THEN 'INVALID'
   WHEN EXISTS(
    SELECT 1 FROM finance.upload_intent u
    JOIN target x ON x.revision_id=u.revision_id
    WHERE (u.tenant_id,u.company_id,u.dossier_id,u.actor_id,u.document_kind)=
          (t,c,d,a,ci.document_kind)
     AND u.state<>'CANCELLED'
   ) THEN 'UNKNOWN'
   WHEN ci.mandatory THEN 'MISSING'
   ELSE 'NOT_APPLICABLE'
  END,
  ci.clause_id,ARRAY[]::uuid[]
 FROM finance.checklist_item ci,run
 WHERE (ci.tenant_id,ci.company_id,ci.policy_id)=(t,c,run.policy_id)
  AND ci.id>after_id
 ORDER BY ci.id
 LIMIT n
$$;
ALTER FUNCTION finance.runtime_checklist(uuid,uuid,uuid,uuid,uuid,integer) OWNER TO finance_checklist_reader;
REVOKE ALL ON FUNCTION finance.runtime_checklist(uuid,uuid,uuid,uuid,uuid,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_checklist(uuid,uuid,uuid,uuid,uuid,integer) TO finance_runtime;
