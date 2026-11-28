-- Pre-seal and multi-resource read models. The amount is a single hold
-- applied to each policy-required cap, not a sum across cap dimensions.
GRANT SELECT ON finance.membership TO finance_reservation_reader;
CREATE POLICY reservation_reader_membership ON finance.membership FOR SELECT TO finance_reservation_reader
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE FUNCTION finance.runtime_reservation_ledger(t uuid,c uuid,d uuid,a uuid,after_id uuid,n integer)
RETURNS TABLE(id uuid,snapshot_id uuid,resource_id uuid,resource_ids uuid[],revision_id uuid,policy_id uuid,amount text,remaining text,currency text,state text,version bigint)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF (t,c,a) IS DISTINCT FROM (finance.scope_tenant(),finance.scope_company(),finance.scope_actor())
 OR d IS NULL OR after_id IS NULL OR n IS NULL OR n NOT BETWEEN 1 AND 100
 OR NOT (finance.field_access_allowed(t,c,d,a) OR EXISTS(
 SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a) AND m.role='FINANCE_APPROVER'
 AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())))
 THEN RAISE EXCEPTION 'reservation read forbidden' USING ERRCODE='42501'; END IF;
 RETURN QUERY SELECT r.id,s.id,r.resource_ids[1],r.resource_ids,r.revision_id,r.binding_policy_id,
 r.amount::text,r.remaining::text,r.currency,r.state,
 (SELECT count(*)::bigint FROM finance.allocation_effect e WHERE (e.tenant_id,e.company_id,e.reservation_id)=(t,c,r.id))
 FROM finance.reservation r JOIN finance.dossier_revision dr ON (dr.tenant_id,dr.company_id,dr.id)=(r.tenant_id,r.company_id,r.revision_id)
 LEFT JOIN LATERAL (SELECT ds.id FROM finance.dossier_snapshot ds WHERE (ds.tenant_id,ds.company_id,ds.dossier_id,ds.revision_id,ds.reservation_id)=(t,c,d,r.revision_id,r.id)
 ORDER BY ds.sealed_at DESC,ds.id DESC LIMIT 1) s ON true
 WHERE (r.tenant_id,r.company_id,dr.dossier_id)=(t,c,d) AND r.id>after_id ORDER BY r.id LIMIT n;
END $$;
ALTER FUNCTION finance.runtime_reservation_ledger(uuid,uuid,uuid,uuid,uuid,integer) OWNER TO finance_reservation_reader;
REVOKE ALL ON FUNCTION finance.runtime_reservation_ledger(uuid,uuid,uuid,uuid,uuid,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_reservation_ledger(uuid,uuid,uuid,uuid,uuid,integer) TO finance_runtime;

-- Independently authorized review of the exact draft, not just its UUID.
CREATE FUNCTION finance.runtime_reference_import_material(t uuid,c uuid,a uuid,i uuid)
RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE result jsonb;
BEGIN
 IF (t,c,a) IS DISTINCT FROM (finance.scope_tenant(),finance.scope_company(),finance.scope_actor()) THEN RAISE EXCEPTION 'scope' USING ERRCODE='42501'; END IF;
 SELECT jsonb_build_object('id',r.id,'document_id',r.source_document_id,'source_digest',encode(r.source_digest,'hex'),'kind',split_part(r.external_version,':',1),'coverage',split_part(r.external_version,':',2),'as_of',r.as_of,'fresh_until',r.fresh_until,'state',r.state,'records',r.import_payload,'evidence_ids',r.import_evidence,'maker_id',r.maker_id)
 INTO result FROM finance.reference_snapshot r WHERE (r.tenant_id,r.company_id,r.id)=(t,c,i) AND r.source_document_id IS NOT NULL
 AND EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a) AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())
 AND (m.role='FINANCE_APPROVER' OR (r.maker_id=a AND m.role IN ('REQUESTER','POLICY_ADMIN'))));
 RETURN result;
END $$;
ALTER FUNCTION finance.runtime_reference_import_material(uuid,uuid,uuid,uuid) OWNER TO finance_reference_writer;
REVOKE ALL ON FUNCTION finance.runtime_reference_import_material(uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_reference_import_material(uuid,uuid,uuid,uuid) TO finance_runtime;

-- Only source-bound resource IDs (not snapshot IDs) may be offered for a hold.
GRANT SELECT ON finance.reservation,finance.reservation_line,finance.source_coverage TO finance_reference_writer;
CREATE POLICY reference_hold_read ON finance.reservation FOR SELECT TO finance_reference_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY reference_hold_line_read ON finance.reservation_line FOR SELECT TO finance_reference_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY reference_coverage_read_bound ON finance.source_coverage FOR SELECT TO finance_reference_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE FUNCTION finance.runtime_reservable_resources(t uuid,c uuid,a uuid,after_id uuid,n integer)
RETURNS TABLE(id uuid,kind text,source_key text,snapshot_id uuid,currency text,cap text,consumed text,held text,available text,fresh_until text)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF (t,c,a) IS DISTINCT FROM (finance.scope_tenant(),finance.scope_company(),finance.scope_actor())
 OR after_id IS NULL OR n IS NULL OR n NOT BETWEEN 1 AND 100
 OR NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a) AND m.role='FINANCE_APPROVER'
 AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 THEN RAISE EXCEPTION 'finance approver required' USING ERRCODE='42501'; END IF;
 RETURN QUERY SELECT f.id,f.resource_kind,f.source_key,f.snapshot_id,f.currency,f.cap::text,f.consumed::text,h.amount::text,
 (f.cap-f.consumed-h.amount)::text,to_char(f.fresh_until AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"')
 FROM finance.financial_resource f JOIN finance.reference_snapshot s ON (s.tenant_id,s.company_id,s.id)=(f.tenant_id,f.company_id,f.snapshot_id)
 JOIN finance.source_coverage sc ON (sc.tenant_id,sc.company_id,sc.snapshot_id,sc.resource_kind)=(f.tenant_id,f.company_id,f.snapshot_id,f.resource_kind)
 CROSS JOIN LATERAL(SELECT coalesce(sum(r.remaining),0) amount FROM finance.reservation_line l JOIN finance.reservation r ON (r.tenant_id,r.company_id,r.id)=(l.tenant_id,l.company_id,l.reservation_id)
 WHERE (l.tenant_id,l.company_id,l.resource_id)=(t,c,f.id) AND r.state IN ('RESERVED','COMMITTED','PARTIAL')) h
 WHERE (f.tenant_id,f.company_id)=(t,c) AND f.id>after_id AND f.coverage_complete AND sc.complete AND s.state='APPROVED' AND s.source_document_id IS NOT NULL AND f.fresh_until>statement_timestamp() AND s.fresh_until>statement_timestamp()
 ORDER BY f.id LIMIT n;
END $$;
ALTER FUNCTION finance.runtime_reservable_resources(uuid,uuid,uuid,uuid,integer) OWNER TO finance_reference_writer;
REVOKE ALL ON FUNCTION finance.runtime_reservable_resources(uuid,uuid,uuid,uuid,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_reservable_resources(uuid,uuid,uuid,uuid,integer) TO finance_runtime;
