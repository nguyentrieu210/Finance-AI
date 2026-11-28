-- E12: accepted receipt identities are approved reference data, not arbitrary
-- document UUIDs. Claims are immutable at seal; never release on an unverified cancellation.
CREATE TABLE finance.acceptance_claim (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,snapshot_id uuid NOT NULL,
 contract_key text NOT NULL,line_key text NOT NULL,quantity numeric(28,8) NOT NULL CHECK(quantity>0),
 reconciliation_id uuid NOT NULL,source_snapshot_id uuid NOT NULL,
 PRIMARY KEY(tenant_id,company_id,snapshot_id,contract_key,line_key),
 FOREIGN KEY(tenant_id,company_id,snapshot_id) REFERENCES finance.dossier_snapshot(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,reconciliation_id) REFERENCES finance.line_reconciliation(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,source_snapshot_id) REFERENCES finance.reference_snapshot(tenant_id,company_id,id)
);
ALTER TABLE finance.acceptance_claim ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.acceptance_claim FORCE ROW LEVEL SECURITY;
CREATE TRIGGER acceptance_claim_immutable BEFORE UPDATE OR DELETE ON finance.acceptance_claim FOR EACH ROW EXECUTE FUNCTION finance.reject_allocation_mutation();
GRANT SELECT,INSERT ON finance.acceptance_claim TO finance_workflow_writer;
GRANT SELECT ON finance.acceptance_claim TO finance_reference_writer;
GRANT SELECT ON finance.line_reconciliation TO finance_workflow_writer;
CREATE POLICY claim_workflow_read ON finance.acceptance_claim FOR SELECT TO finance_workflow_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY claim_workflow_insert ON finance.acceptance_claim FOR INSERT TO finance_workflow_writer WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY claim_reference_read ON finance.acceptance_claim FOR SELECT TO finance_reference_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY reconciliation_workflow_read ON finance.line_reconciliation FOR SELECT TO finance_workflow_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());

CREATE FUNCTION finance.valid_acceptance_lines(lines jsonb) RETURNS boolean
LANGUAGE plpgsql IMMUTABLE SET search_path=pg_catalog AS $$
DECLARE ln jsonb;
BEGIN
 IF jsonb_typeof(lines) IS DISTINCT FROM 'array' OR jsonb_array_length(lines)>256 THEN RETURN false; END IF;
 IF (SELECT count(DISTINCT x->>'key') FROM jsonb_array_elements(lines) x)<>jsonb_array_length(lines) THEN RETURN false; END IF;
 FOR ln IN SELECT value FROM jsonb_array_elements(lines) LOOP
  PERFORM finance.reference_text(ln,'key');PERFORM finance.reference_text(ln,'item');PERFORM finance.reference_text(ln,'unit');
  IF finance.reference_nonnegative(ln,'quantity')<=0 OR finance.reference_nonnegative(ln,'unit_price')*finance.reference_nonnegative(ln,'quantity')<>finance.reference_nonnegative(ln,'value') THEN RETURN false; END IF;
 END LOOP;
 RETURN true;
END $$;
REVOKE ALL ON FUNCTION finance.valid_acceptance_lines(jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.valid_acceptance_lines(jsonb) TO finance_reference_writer,finance_workflow_writer;
CREATE FUNCTION finance.assert_acceptance_refresh(t uuid,c uuid,ck text,detail jsonb) RETURNS void
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
BEGIN
 IF EXISTS(SELECT 1 FROM (SELECT ac.line_key,sum(ac.quantity) used FROM finance.acceptance_claim ac WHERE (ac.tenant_id,ac.company_id,ac.contract_key)=(t,c,ck) GROUP BY ac.line_key) q
 LEFT JOIN LATERAL(SELECT x FROM jsonb_array_elements(COALESCE(detail->'acceptance_lines','[]')) x WHERE x->>'key'=q.line_key) ln ON true
 WHERE ln.x IS NULL OR (ln.x->>'quantity')::numeric<q.used)
 THEN RAISE EXCEPTION 'refresh would erase or overdraw a claimed acceptance line' USING ERRCODE='23514'; END IF;
 -- Once billed, the source's identity, UOM and unit price must remain traceable.
 IF EXISTS(SELECT 1 FROM finance.acceptance_claim ac JOIN finance.reference_snapshot s ON (s.tenant_id,s.company_id,s.id)=(ac.tenant_id,ac.company_id,ac.source_snapshot_id)
 CROSS JOIN LATERAL jsonb_array_elements(s.import_payload) oldrow CROSS JOIN LATERAL jsonb_array_elements(oldrow->'details'->'acceptance_lines') oldln
 JOIN LATERAL (SELECT x FROM jsonb_array_elements(COALESCE(detail->'acceptance_lines','[]')) x WHERE x->>'key'=ac.line_key) n ON true
 WHERE (ac.tenant_id,ac.company_id,ac.contract_key)=(t,c,ck) AND oldrow->'details'->>'contract_key'=ck AND oldln->>'key'=ac.line_key
 AND (oldln->>'item',oldln->>'unit',(oldln->>'unit_price')::numeric) IS DISTINCT FROM (n.x->>'item',n.x->>'unit',(n.x->>'unit_price')::numeric))
 THEN RAISE EXCEPTION 'claimed acceptance identity changed' USING ERRCODE='23514'; END IF;
END $$;
REVOKE ALL ON FUNCTION finance.assert_acceptance_refresh(uuid,uuid,text,jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.assert_acceptance_refresh(uuid,uuid,text,jsonb) TO finance_reference_writer;

-- A match is eligible only against the current clean evidence and independently
-- verified reference schedule. Its quantities are checked again when sealing.
CREATE FUNCTION finance.verified_acceptance_material(t uuid,c uuid,d uuid,rev uuid,pid uuid,hold_id uuid)
RETURNS jsonb LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE m finance.line_reconciliation%ROWTYPE; fr finance.financial_resource%ROWTYPE; ln jsonb; canonical jsonb; used numeric; allocated numeric; ev jsonb; claims jsonb:='[]'; ck text; row_data jsonb; source_ev jsonb; line_ev jsonb; total numeric:=0;
BEGIN
 SELECT x.* INTO fr FROM finance.financial_resource x JOIN finance.reservation_line l ON (l.tenant_id,l.company_id,l.resource_id)=(x.tenant_id,x.company_id,x.id)
 WHERE (l.tenant_id,l.company_id,l.reservation_id,x.resource_kind)=(t,c,hold_id,'CONTRACT');
 IF NOT FOUND THEN RETURN jsonb_build_object('required',false,'valid',true,'claims','[]'::jsonb,'facts','[]'::jsonb); END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.policy_version p WHERE (p.tenant_id,p.company_id,p.id)=(t,c,pid) AND p.finance_gate ? 'line_match') THEN RETURN NULL; END IF;
 ck:=fr.reference_details->>'contract_key';
 IF ck IS NULL OR NOT COALESCE(finance.valid_acceptance_lines(fr.reference_details->'acceptance_lines'),false) THEN RETURN NULL; END IF;
 SELECT x.* INTO m FROM finance.line_reconciliation x WHERE (x.tenant_id,x.company_id,x.dossier_id,x.revision_id,x.policy_id)=(t,c,d,rev,pid) ORDER BY x.dossier_version DESC,x.id DESC LIMIT 1;
 IF NOT FOUND OR m.result->>'outcome'<>'MATCHED' OR m.material->>'coverage_complete'<>'true' THEN RETURN NULL; END IF;
 -- No stale OCR references, replaced uploads or cross-revision evidence.
 FOR ln IN SELECT value FROM jsonb_array_elements((m.material->'sources')||(m.material->'targets')) LOOP
  IF NOT EXISTS(SELECT 1 FROM finance.evidence e JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(e.tenant_id,e.company_id,e.ocr_run_id)
  JOIN finance.document_ref dr ON (dr.tenant_id,dr.company_id,dr.id)=(o.tenant_id,o.company_id,o.document_id)
  JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(dr.tenant_id,dr.company_id,dr.id)
  JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id)=(dr.tenant_id,dr.company_id,dr.id)
  WHERE (e.tenant_id,e.company_id,e.id)=(t,c,(ln->>'evidence_id')::uuid) AND (dr.dossier_id,dr.revision_id)=(d,rev)
  AND b.scan_state='CLEAN' AND b.generation=o.generation AND b.sha256_digest=o.source_digest AND u.state='FINALIZED' AND u.generation=b.generation AND b.sha256_digest=u.declared_sha256
  AND NOT EXISTS(SELECT 1 FROM finance.ocr_run n WHERE (n.tenant_id,n.company_id,n.document_id,n.revision_id)=(o.tenant_id,o.company_id,o.document_id,o.revision_id) AND (n.created_at,n.id)>(o.created_at,o.id)))
  THEN RETURN NULL; END IF;
 END LOOP;
 SELECT item INTO row_data FROM finance.reference_snapshot s CROSS JOIN LATERAL jsonb_array_elements(s.import_payload) item WHERE (s.tenant_id,s.company_id,s.id)=(t,c,fr.snapshot_id) AND jsonb_build_array('MANUAL_API',item->>'source_key')::text=fr.source_key;
 IF row_data IS NULL THEN RETURN NULL; END IF;
 SELECT coalesce(jsonb_agg(DISTINCT x ORDER BY x),'[]') INTO ev FROM (
 SELECT to_jsonb(ln->>'evidence_id') x FROM jsonb_array_elements((m.material->'sources')||(m.material->'targets')) ln
 UNION ALL SELECT x FROM jsonb_array_elements(row_data->'evidence_ids') x) e;
 IF jsonb_array_length(ev) NOT BETWEEN 1 AND 64 THEN RETURN NULL; END IF;
 FOR ln IN SELECT value FROM jsonb_array_elements(m.material->'sources') ORDER BY value->>'id' LOOP
  SELECT x INTO canonical FROM jsonb_array_elements(fr.reference_details->'acceptance_lines') x WHERE x->>'key'=ln->>'id';
  IF canonical IS NULL OR (canonical->>'item',canonical->>'unit',(canonical->>'quantity')::numeric,(canonical->>'unit_price')::numeric,(canonical->>'value')::numeric)
  IS DISTINCT FROM (ln->>'item',ln->>'unit',(ln->>'quantity')::numeric,(ln->>'unit_price')::numeric,(ln->>'value')::numeric) OR ln->>'currency'<>fr.currency THEN RETURN NULL; END IF;
  SELECT coalesce(sum((x->>'source_quantity')::numeric),0) INTO allocated FROM jsonb_array_elements(m.material->'allocations') x WHERE x->>'source'=ln->>'id';
  IF allocated=0 THEN CONTINUE; END IF;
  SELECT coalesce(sum(x.quantity),0) INTO used FROM finance.acceptance_claim x WHERE (x.tenant_id,x.company_id,x.contract_key,x.line_key)=(t,c,ck,ln->>'id');
  IF used+allocated>(canonical->>'quantity')::numeric THEN RETURN NULL; END IF;
  total:=total+allocated*(canonical->>'unit_price')::numeric;
  claims:=claims||jsonb_build_array(jsonb_build_object('contract',ck,'line',ln->>'id','quantity',allocated,'reconciliation',m.id,'source_snapshot',fr.snapshot_id));
 END LOOP;
 IF jsonb_array_length(claims)=0 THEN RETURN NULL; END IF;
 RETURN jsonb_build_object('required',true,'valid',true,'claims',claims,'facts',jsonb_build_array(
 jsonb_build_object('path','matching.complete','value','true','verified',true,'evidence',ev),
 jsonb_build_object('path','matching.allocated_value','value',total::text,'currency',fr.currency,'verified',true,'evidence',ev),
 jsonb_build_object('path','matching.line_count','value',jsonb_array_length(claims)::text,'verified',true,'evidence',ev)));
END $$;
REVOKE ALL ON FUNCTION finance.verified_acceptance_material(uuid,uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.verified_acceptance_material(uuid,uuid,uuid,uuid,uuid,uuid) TO finance_workflow_writer;
CREATE FUNCTION finance.seal_acceptance_claims(t uuid,c uuid,d uuid,rev uuid,pid uuid,hold_id uuid,snap uuid) RETURNS void
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE mat jsonb;
BEGIN
 -- Serializes claims across installments and dossiers. Existing resource locks
 -- are held by the seal transaction; all claims are written atomically with it.
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.acceptance.claim/v1/'||t::text||'/'||c::text,0));
 mat:=finance.verified_acceptance_material(t,c,d,rev,pid,hold_id);
 IF mat IS NULL THEN RAISE EXCEPTION 'acceptance unavailable or already claimed' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.acceptance_claim(tenant_id,company_id,snapshot_id,contract_key,line_key,quantity,reconciliation_id,source_snapshot_id)
 SELECT t,c,snap,x->>'contract',x->>'line',(x->>'quantity')::numeric,(x->>'reconciliation')::uuid,(x->>'source_snapshot')::uuid FROM jsonb_array_elements(mat->'claims') x;
END $$;
REVOKE ALL ON FUNCTION finance.seal_acceptance_claims(uuid,uuid,uuid,uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.seal_acceptance_claims(uuid,uuid,uuid,uuid,uuid,uuid,uuid) TO finance_workflow_writer;
