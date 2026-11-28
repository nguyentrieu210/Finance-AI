-- E13: extra payment is a separately authorized payment against the exact
-- sealed and accepted settlement, not additional consumption of its advances.
CREATE TABLE finance.settlement_extra_claim(
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,settlement_snapshot_id uuid NOT NULL,payment_snapshot_id uuid NOT NULL,
 amount numeric(28,8) NOT NULL CHECK(amount>0),currency text NOT NULL CHECK(currency~'^[A-Z]{3}$'),
 PRIMARY KEY(tenant_id,company_id,settlement_snapshot_id),
 UNIQUE(tenant_id,company_id,payment_snapshot_id),
 FOREIGN KEY(tenant_id,company_id,settlement_snapshot_id) REFERENCES finance.dossier_snapshot(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,payment_snapshot_id) REFERENCES finance.dossier_snapshot(tenant_id,company_id,id)
);
ALTER TABLE finance.settlement_extra_claim ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.settlement_extra_claim FORCE ROW LEVEL SECURITY;
CREATE TRIGGER settlement_extra_immutable BEFORE UPDATE OR DELETE ON finance.settlement_extra_claim FOR EACH ROW EXECUTE FUNCTION finance.reject_allocation_mutation();
GRANT SELECT,INSERT ON finance.settlement_extra_claim TO finance_workflow_writer;
CREATE POLICY settlement_extra_read ON finance.settlement_extra_claim FOR SELECT TO finance_workflow_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY settlement_extra_insert ON finance.settlement_extra_claim FOR INSERT TO finance_workflow_writer WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE FUNCTION finance.verified_extra_settlement(t uuid,c uuid,d uuid,fields jsonb,amount_row jsonb,cur text,n numeric)
RETURNS jsonb LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE link jsonb; beneficiary jsonb; snap finance.dossier_snapshot%ROWTYPE; original_extra jsonb; original_currency jsonb; original_beneficiary jsonb; ev jsonb; sid uuid;
BEGIN
 SELECT value INTO link FROM jsonb_array_elements(fields) WHERE value->>'path'='payment_request.settlement_snapshot';
 SELECT value INTO beneficiary FROM jsonb_array_elements(fields) WHERE value->>'path'='payment_request.beneficiary_key';
 IF link IS NULL OR beneficiary IS NULL OR link->>'verified'<>'true' OR beneficiary->>'verified'<>'true'
 OR link->>'document' IS DISTINCT FROM amount_row->>'document' OR beneficiary->>'document' IS DISTINCT FROM amount_row->>'document'
 THEN RETURN NULL; END IF;
 BEGIN sid:=(link->>'value')::uuid; EXCEPTION WHEN invalid_text_representation THEN RETURN NULL; END;
 SELECT s.* INTO snap FROM finance.dossier_snapshot s JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(s.tenant_id,s.company_id,s.dossier_id)
 JOIN finance.dossier_revision dr ON (dr.tenant_id,dr.company_id,dr.id)=(s.tenant_id,s.company_id,s.revision_id)
 WHERE (s.tenant_id,s.company_id,s.id)=(t,c,sid) AND ds.kind='SETTLEMENT' AND ds.status='ACCEPTED' AND ds.id<>d AND dr.number=ds.revision;
 IF NOT FOUND OR EXISTS(SELECT 1 FROM finance.settlement_extra_claim x WHERE (x.tenant_id,x.company_id,x.settlement_snapshot_id)=(t,c,sid)) THEN RETURN NULL; END IF;
 SELECT value INTO original_extra FROM jsonb_array_elements(snap.export_material->'fields') WHERE value->>'path'='advance_settlement.extra';
 SELECT value INTO original_currency FROM jsonb_array_elements(snap.export_material->'fields') WHERE value->>'path'='advance_settlement.currency';
 SELECT value INTO original_beneficiary FROM jsonb_array_elements(snap.export_material->'fields') WHERE value->>'path'='advance_settlement.beneficiary_key';
 IF original_extra IS NULL OR original_currency IS NULL OR original_beneficiary IS NULL OR original_extra->>'value' !~ '^[0-9]{1,20}([.][0-9]{1,8})?$'
 OR (original_extra->>'value')::numeric IS DISTINCT FROM n OR original_currency->>'value' IS DISTINCT FROM cur
 OR original_beneficiary->>'value' IS DISTINCT FROM beneficiary->>'value'
 OR (SELECT count(*) FROM jsonb_array_elements(snap.export_material->'fields') x WHERE x->>'path' IN ('advance_settlement.extra','advance_settlement.currency','advance_settlement.beneficiary_key'))<>3
 THEN RETURN NULL; END IF;
 SELECT coalesce(jsonb_agg(DISTINCT x ORDER BY x),'[]') INTO ev FROM jsonb_array_elements((link->'evidence')||(beneficiary->'evidence')||(amount_row->'evidence')||(original_extra->'evidence')||(original_currency->'evidence')||(original_beneficiary->'evidence')) x;
 IF jsonb_array_length(ev) NOT BETWEEN 1 AND 64 THEN RETURN NULL; END IF;
 RETURN jsonb_build_object('snapshot',sid,'amount',n,'currency',cur,'evidence',ev);
END $$;
REVOKE ALL ON FUNCTION finance.verified_extra_settlement(uuid,uuid,uuid,jsonb,jsonb,text,numeric) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.verified_extra_settlement(uuid,uuid,uuid,jsonb,jsonb,text,numeric) TO finance_workflow_writer;

CREATE FUNCTION finance.seal_extra_settlement(t uuid,c uuid,d uuid,rev uuid,pid uuid,hold_id uuid,v bigint,snap uuid)
RETURNS void LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE hold finance.reservation%ROWTYPE; mat jsonb; extra jsonb; sid uuid;
BEGIN
 SELECT * INTO STRICT hold FROM finance.reservation x WHERE (x.tenant_id,x.company_id,x.id)=(t,c,hold_id);
 mat:=finance.bound_finance_material(t,c,d,rev,pid,v,hold.resource_ids);
 IF mat IS NULL THEN RAISE EXCEPTION 'missing sealed financial material' USING ERRCODE='23514'; END IF;
 extra:=mat->'extra_settlement';
 IF extra IS NULL OR extra='null'::jsonb THEN RETURN; END IF;
 sid:=(extra->>'snapshot')::uuid;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.settlement.extra/v1/'||t::text||'/'||c::text||'/'||sid::text,0));
 -- Unique source settlement claim is the final race guard; no second payment
 -- can seal against the same extra balance, even with different idempotency keys.
 INSERT INTO finance.settlement_extra_claim(tenant_id,company_id,settlement_snapshot_id,payment_snapshot_id,amount,currency)
 VALUES(t,c,sid,snap,(extra->>'amount')::numeric,extra->>'currency');
END $$;
REVOKE ALL ON FUNCTION finance.seal_extra_settlement(uuid,uuid,uuid,uuid,uuid,uuid,bigint,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.seal_extra_settlement(uuid,uuid,uuid,uuid,uuid,uuid,bigint,uuid) TO finance_workflow_writer;
