-- E21/E23: grant approvers visibility only to parent rows required by a
-- scoped waiver or reconciliation case, without recursive runtime RLS.
CREATE ROLE finance_case_reader NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_case_reader;
GRANT SELECT ON finance.membership,finance.waiver,finance.reconciliation_case TO finance_case_reader;
CREATE POLICY case_reader_membership ON finance.membership FOR SELECT TO finance_case_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY case_reader_waiver ON finance.waiver FOR SELECT TO finance_case_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY case_reader_reconciliation ON finance.reconciliation_case FOR SELECT TO finance_case_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());

CREATE FUNCTION finance.approver_case_parent(t uuid,c uuid,d uuid,rev uuid,h uuid)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT t=finance.scope_tenant() AND c=finance.scope_company()
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=t AND m.company_id=c
  AND m.actor_id=finance.scope_actor() AND m.role='FINANCE_APPROVER' AND m.active
  AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 AND (
  (d IS NOT NULL AND h IS NULL AND EXISTS(
   SELECT 1 FROM finance.waiver w WHERE w.tenant_id=t AND w.company_id=c AND w.dossier_id=d
    AND (rev IS NULL OR w.revision_id=rev)))
  OR (d IS NULL AND rev IS NULL AND h IS NOT NULL AND EXISTS(
   SELECT 1 FROM finance.reconciliation_case r WHERE r.tenant_id=t AND r.company_id=c AND r.handoff_id=h))
 )
$$;
ALTER FUNCTION finance.approver_case_parent(uuid,uuid,uuid,uuid,uuid) OWNER TO finance_case_reader;
REVOKE ALL ON FUNCTION finance.approver_case_parent(uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.approver_case_parent(uuid,uuid,uuid,uuid,uuid) TO finance_runtime;

CREATE POLICY dossier_waiver_approver_read ON finance.dossier FOR SELECT TO finance_runtime
 USING(finance.approver_case_parent(tenant_id,company_id,id,NULL,NULL));
CREATE POLICY revision_waiver_approver_read ON finance.dossier_revision FOR SELECT TO finance_runtime
 USING(finance.approver_case_parent(tenant_id,company_id,dossier_id,id,NULL));
CREATE POLICY snapshot_waiver_approver_read ON finance.dossier_snapshot FOR SELECT TO finance_runtime
 USING(finance.approver_case_parent(tenant_id,company_id,dossier_id,revision_id,NULL));
CREATE POLICY handoff_reconciliation_approver_read ON finance.handoff FOR SELECT TO finance_runtime
 USING(finance.approver_case_parent(tenant_id,company_id,NULL,NULL,id));
