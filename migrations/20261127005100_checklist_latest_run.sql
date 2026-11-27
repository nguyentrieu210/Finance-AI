-- E15: select the latest run before checking policy availability; never fall back to stale policy.
CREATE OR REPLACE FUNCTION finance.runtime_checklist(
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
   AND t=finance.scope_tenant() AND c=finance.scope_company()
   AND a=finance.scope_actor() AND n BETWEEN 1 AND 100
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
   ELSE 'UNKNOWN'
  END,
  ci.clause_id,ARRAY[]::uuid[]
 FROM finance.checklist_item ci,run
 WHERE (ci.tenant_id,ci.company_id,ci.policy_id)=(t,c,run.policy_id)
  AND ci.id>after_id
 ORDER BY ci.id
 LIMIT CASE WHEN n BETWEEN 1 AND 100 THEN n ELSE 0 END
$$;
