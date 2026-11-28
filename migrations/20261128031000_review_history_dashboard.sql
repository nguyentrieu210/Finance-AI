-- E24 read-only aggregate of immutable accounting decisions. No new table,
-- business approval or write privilege; use existing E18 owner-scoped port.
CREATE FUNCTION finance.runtime_review_history_counts(t uuid,c uuid,a uuid)
RETURNS TABLE(accepted bigint, first_pass bigint, returned_cases bigint, unknown_accepted bigint)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL
 OR t IS DISTINCT FROM finance.scope_tenant()
 OR c IS DISTINCT FROM finance.scope_company()
 OR a IS DISTINCT FROM finance.scope_actor()
 OR NOT EXISTS(SELECT 1 FROM finance.membership m
   WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a)
   AND m.role='REQUESTER' AND m.active
   AND m.valid_from<=statement_timestamp()
   AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 THEN RAISE EXCEPTION 'requester required' USING ERRCODE='42501'; END IF;

 RETURN QUERY
 WITH owned AS (
   SELECT d.id,d.status FROM finance.dossier d
   WHERE (d.tenant_id,d.company_id,d.owner_id)=(t,c,a)
 ), histories AS (
   SELECT s.dossier_id,
     count(*) FILTER(WHERE r.decision='ACCEPTED') AS approvals,
     count(*) FILTER(WHERE r.decision='RETURNED') AS return_count,
     count(*) AS decisions,
     count(*) FILTER(WHERE ar.id IS NOT NULL) AS matched
   FROM finance.dossier_snapshot s
   JOIN owned d ON d.id=s.dossier_id
   JOIN finance.review_decision r
     ON (r.tenant_id,r.company_id,r.snapshot_id)=(s.tenant_id,s.company_id,s.id)
   LEFT JOIN finance.accounting_review ar
     ON (ar.tenant_id,ar.company_id,ar.snapshot_id,ar.reviewer_id,ar.state)
       =(r.tenant_id,r.company_id,r.snapshot_id,r.actor_id,r.decision)
     AND ar.dossier_id=s.dossier_id
   WHERE (s.tenant_id,s.company_id)=(t,c)
   GROUP BY s.dossier_id
 ), classified AS (
   SELECT d.status,
     coalesce(h.return_count,0) AS return_count,
     coalesce(h.approvals=1 AND h.decisions=h.matched,false) AS verified_accept
   FROM owned d LEFT JOIN histories h ON h.dossier_id=d.id
 )
 SELECT count(*) FILTER(WHERE status='ACCEPTED' AND verified_accept),
        count(*) FILTER(WHERE status='ACCEPTED' AND verified_accept AND return_count=0),
        count(*) FILTER(WHERE return_count>0),
        count(*) FILTER(WHERE status='ACCEPTED' AND NOT verified_accept)
 FROM classified;
END $$;
ALTER FUNCTION finance.runtime_review_history_counts(uuid,uuid,uuid) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.runtime_review_history_counts(uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_review_history_counts(uuid,uuid,uuid) TO finance_runtime;
