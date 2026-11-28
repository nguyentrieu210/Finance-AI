-- E23/E12/E13/E14 synthetic reference import, no real ERP access or runtime grant.
CREATE TABLE finance.reference_snapshot (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 source_system text NOT NULL CHECK(source_system ~ '^[A-Z][A-Z0-9_]{0,63}$'),
 external_version text NOT NULL CHECK(octet_length(external_version) BETWEEN 1 AND 128),
 source_digest bytea NOT NULL CHECK(octet_length(source_digest)=32),
 as_of timestamptz NOT NULL, fresh_until timestamptz NOT NULL,
 maker_id uuid NOT NULL, checker_id uuid,
 state text NOT NULL DEFAULT 'DRAFT' CHECK(state IN ('DRAFT','APPROVED','REVOKED')),
 PRIMARY KEY(tenant_id,company_id,id),
 UNIQUE(tenant_id,company_id,source_system,external_version),
 FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id),
 FOREIGN KEY(tenant_id,maker_id) REFERENCES finance.identity_subject(tenant_id,id),
 FOREIGN KEY(tenant_id,checker_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(fresh_until>as_of),
 CHECK((state='DRAFT' AND checker_id IS NULL) OR (state IN ('APPROVED','REVOKED') AND checker_id IS NOT NULL AND checker_id<>maker_id))
);
CREATE TABLE finance.source_coverage (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, snapshot_id uuid NOT NULL,
 resource_kind text NOT NULL CHECK(resource_kind IN ('CONTRACT','INVOICE','ADVANCE','BUDGET')),
 complete boolean NOT NULL,
 watermark text NOT NULL CHECK(octet_length(watermark) BETWEEN 1 AND 128),
 PRIMARY KEY(tenant_id,company_id,snapshot_id,resource_kind),
 FOREIGN KEY(tenant_id,company_id,snapshot_id) REFERENCES finance.reference_snapshot(tenant_id,company_id,id)
);
CREATE TABLE finance.vendor_ref (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, snapshot_id uuid NOT NULL,id uuid NOT NULL,
 external_key text NOT NULL CHECK(octet_length(external_key) BETWEEN 1 AND 128),
 name text NOT NULL CHECK(octet_length(name) BETWEEN 1 AND 512),
 PRIMARY KEY(tenant_id,company_id,snapshot_id,id),
 UNIQUE(tenant_id,company_id,snapshot_id,external_key),
 FOREIGN KEY(tenant_id,company_id,snapshot_id) REFERENCES finance.reference_snapshot(tenant_id,company_id,id)
);
CREATE TABLE finance.contract_ref (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,snapshot_id uuid NOT NULL,id uuid NOT NULL,
 external_key text NOT NULL CHECK(octet_length(external_key) BETWEEN 1 AND 128),
 vendor_id uuid NOT NULL,currency text NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),
 cap numeric(28,8) NOT NULL CHECK(cap>=0), consumed numeric(28,8) NOT NULL CHECK(consumed>=0 AND consumed<=cap),
 effective_from date NOT NULL,effective_until date NOT NULL CHECK(effective_until>effective_from),
 PRIMARY KEY(tenant_id,company_id,snapshot_id,id),
 UNIQUE(tenant_id,company_id,snapshot_id,external_key),
 FOREIGN KEY(tenant_id,company_id,snapshot_id,vendor_id) REFERENCES finance.vendor_ref(tenant_id,company_id,snapshot_id,id)
);
CREATE TABLE finance.invoice_ref (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,snapshot_id uuid NOT NULL,id uuid NOT NULL,
 vendor_id uuid NOT NULL,series text NOT NULL CHECK(octet_length(series) BETWEEN 1 AND 64),
 invoice_number text NOT NULL CHECK(octet_length(invoice_number) BETWEEN 1 AND 128),
 currency text NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),
 cap numeric(28,8) NOT NULL CHECK(cap>=0), consumed numeric(28,8) NOT NULL CHECK(consumed>=0 AND consumed<=cap),
 invoice_date date NOT NULL,
 PRIMARY KEY(tenant_id,company_id,snapshot_id,id),
 UNIQUE(tenant_id,company_id,snapshot_id,vendor_id,series,invoice_number),
 FOREIGN KEY(tenant_id,company_id,snapshot_id,vendor_id) REFERENCES finance.vendor_ref(tenant_id,company_id,snapshot_id,id)
);
CREATE TABLE finance.advance_ref (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,snapshot_id uuid NOT NULL,id uuid NOT NULL,
 external_key text NOT NULL CHECK(octet_length(external_key) BETWEEN 1 AND 128),
 beneficiary_id uuid NOT NULL,currency text NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),
 issued numeric(28,8) NOT NULL CHECK(issued>0), settled numeric(28,8) NOT NULL CHECK(settled>=0),
 refunded numeric(28,8) NOT NULL CHECK(refunded>=0 AND settled+refunded<=issued),
 due_date date NOT NULL,
 PRIMARY KEY(tenant_id,company_id,snapshot_id,id),
 UNIQUE(tenant_id,company_id,snapshot_id,external_key),
 FOREIGN KEY(tenant_id,company_id,snapshot_id) REFERENCES finance.reference_snapshot(tenant_id,company_id,id)
);
CREATE TABLE finance.budget_ref (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,snapshot_id uuid NOT NULL,id uuid NOT NULL,
 cost_center text NOT NULL CHECK(octet_length(cost_center) BETWEEN 1 AND 128),
 account text NOT NULL CHECK(octet_length(account) BETWEEN 1 AND 128),
 currency text NOT NULL CHECK(currency ~ '^[A-Z]{3}$'),
 cap numeric(28,8) NOT NULL CHECK(cap>=0), actual numeric(28,8) NOT NULL CHECK(actual>=0 AND actual<=cap),
 period_start date NOT NULL,period_end date NOT NULL CHECK(period_end>period_start),
 PRIMARY KEY(tenant_id,company_id,snapshot_id,id),
 UNIQUE(tenant_id,company_id,snapshot_id,cost_center,account,period_start,period_end),
 FOREIGN KEY(tenant_id,company_id,snapshot_id) REFERENCES finance.reference_snapshot(tenant_id,company_id,id)
);
ALTER TABLE finance.reference_snapshot ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.reference_snapshot FORCE ROW LEVEL SECURITY;
ALTER TABLE finance.source_coverage ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.source_coverage FORCE ROW LEVEL SECURITY;
ALTER TABLE finance.vendor_ref ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.vendor_ref FORCE ROW LEVEL SECURITY;
ALTER TABLE finance.contract_ref ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.contract_ref FORCE ROW LEVEL SECURITY;
ALTER TABLE finance.invoice_ref ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.invoice_ref FORCE ROW LEVEL SECURITY;
ALTER TABLE finance.advance_ref ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.advance_ref FORCE ROW LEVEL SECURITY;
ALTER TABLE finance.budget_ref ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.budget_ref FORCE ROW LEVEL SECURITY;
CREATE FUNCTION finance.prototype_reference_child_guard() RETURNS trigger
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE t uuid;c uuid;s uuid; st text;
BEGIN
 IF TG_OP='DELETE' THEN t:=OLD.tenant_id;c:=OLD.company_id;s:=OLD.snapshot_id;
 ELSE t:=NEW.tenant_id;c:=NEW.company_id;s:=NEW.snapshot_id; END IF;
 IF TG_OP='UPDATE' AND (NEW.tenant_id,NEW.company_id,NEW.snapshot_id) IS DISTINCT FROM (OLD.tenant_id,OLD.company_id,OLD.snapshot_id)
 THEN RAISE EXCEPTION 'immutable source binding' USING ERRCODE='23514'; END IF;
 SELECT state INTO st FROM finance.reference_snapshot WHERE tenant_id=t AND company_id=c AND id=s FOR UPDATE;
 IF NOT FOUND OR st<>'DRAFT' THEN RAISE EXCEPTION 'frozen source snapshot' USING ERRCODE='23514'; END IF;
 IF TG_OP='DELETE' THEN RETURN OLD; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER source_coverage_freeze BEFORE INSERT OR UPDATE OR DELETE ON finance.source_coverage FOR EACH ROW EXECUTE FUNCTION finance.prototype_reference_child_guard();
CREATE TRIGGER vendor_ref_freeze BEFORE INSERT OR UPDATE OR DELETE ON finance.vendor_ref FOR EACH ROW EXECUTE FUNCTION finance.prototype_reference_child_guard();
CREATE TRIGGER contract_ref_freeze BEFORE INSERT OR UPDATE OR DELETE ON finance.contract_ref FOR EACH ROW EXECUTE FUNCTION finance.prototype_reference_child_guard();
CREATE TRIGGER invoice_ref_freeze BEFORE INSERT OR UPDATE OR DELETE ON finance.invoice_ref FOR EACH ROW EXECUTE FUNCTION finance.prototype_reference_child_guard();
CREATE TRIGGER advance_ref_freeze BEFORE INSERT OR UPDATE OR DELETE ON finance.advance_ref FOR EACH ROW EXECUTE FUNCTION finance.prototype_reference_child_guard();
CREATE TRIGGER budget_ref_freeze BEFORE INSERT OR UPDATE OR DELETE ON finance.budget_ref FOR EACH ROW EXECUTE FUNCTION finance.prototype_reference_child_guard();
ALTER TABLE finance.financial_resource ADD COLUMN snapshot_id uuid, ADD COLUMN resource_kind text, ADD COLUMN source_key text;
ALTER TABLE finance.financial_resource ADD FOREIGN KEY(tenant_id,company_id,snapshot_id,resource_kind) REFERENCES finance.source_coverage(tenant_id,company_id,snapshot_id,resource_kind);
ALTER TABLE finance.financial_resource ADD CHECK((snapshot_id IS NULL AND resource_kind IS NULL AND source_key IS NULL) OR (snapshot_id IS NOT NULL AND resource_kind IS NOT NULL AND source_key IS NOT NULL));
-- Stable logical identity across snapshot versions. No automatic replacement:
-- importing a new balance would risk double counting outstanding local effects.
CREATE UNIQUE INDEX financial_source_identity ON finance.financial_resource(tenant_id,company_id,resource_kind,source_key) WHERE snapshot_id IS NOT NULL;
CREATE FUNCTION finance.prototype_approve_reference(t uuid,c uuid,s uuid,a uuid,day date)
RETURNS void LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE p finance.reference_snapshot%ROWTYPE; n bigint;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' OR day IS NULL THEN RAISE EXCEPTION 'invalid import command' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reference/v1/'||t::text||'/'||c::text,0));
 SELECT * INTO p FROM finance.reference_snapshot WHERE tenant_id=t AND company_id=c AND id=s FOR UPDATE;
 IF NOT FOUND OR p.state<>'DRAFT' OR p.maker_id=a OR a IS NULL
 OR p.as_of>clock_timestamp() OR p.fresh_until<=clock_timestamp()
 THEN RAISE EXCEPTION 'invalid snapshot or checker' USING ERRCODE='23514'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership WHERE tenant_id=t AND company_id=c AND actor_id=a AND role='FINANCE_APPROVER' AND active AND valid_from<=clock_timestamp() AND (valid_until IS NULL OR valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'independent checker required' USING ERRCODE='23514'; END IF;
 -- Explicit complete coverage for every represented kind; no missing default.
 IF EXISTS(SELECT 1 FROM finance.source_coverage WHERE tenant_id=t AND company_id=c AND snapshot_id=s AND NOT complete)
 OR EXISTS(SELECT 1 FROM (SELECT 'CONTRACT' k FROM finance.contract_ref WHERE tenant_id=t AND company_id=c AND snapshot_id=s
 UNION SELECT 'INVOICE' FROM finance.invoice_ref WHERE tenant_id=t AND company_id=c AND snapshot_id=s
 UNION SELECT 'ADVANCE' FROM finance.advance_ref WHERE tenant_id=t AND company_id=c AND snapshot_id=s
 UNION SELECT 'BUDGET' FROM finance.budget_ref WHERE tenant_id=t AND company_id=c AND snapshot_id=s) represented
 WHERE NOT EXISTS(SELECT 1 FROM finance.source_coverage x WHERE x.tenant_id=t AND x.company_id=c AND x.snapshot_id=s AND x.resource_kind=represented.k AND x.complete))
 THEN RAISE EXCEPTION 'incomplete source coverage' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.financial_resource(tenant_id,company_id,id,currency,cap,consumed,coverage_complete,fresh_until,snapshot_id,resource_kind,source_key)
 SELECT t,c,r.id,r.currency,r.cap,r.consumed,true,p.fresh_until,s,'CONTRACT',jsonb_build_array(p.source_system,r.external_key)::text FROM finance.contract_ref r WHERE r.tenant_id=t AND r.company_id=c AND r.snapshot_id=s AND day>=r.effective_from AND day<r.effective_until
 UNION ALL SELECT t,c,r.id,r.currency,r.cap,r.consumed,true,p.fresh_until,s,'INVOICE',jsonb_build_array(p.source_system,v.external_key,r.series,r.invoice_number)::text FROM finance.invoice_ref r JOIN finance.vendor_ref v ON (v.tenant_id,v.company_id,v.snapshot_id,v.id)=(r.tenant_id,r.company_id,r.snapshot_id,r.vendor_id) WHERE r.tenant_id=t AND r.company_id=c AND r.snapshot_id=s AND r.invoice_date<=day
 UNION ALL SELECT t,c,r.id,r.currency,r.issued,r.settled+r.refunded,true,p.fresh_until,s,'ADVANCE',jsonb_build_array(p.source_system,r.external_key)::text FROM finance.advance_ref r WHERE r.tenant_id=t AND r.company_id=c AND r.snapshot_id=s
 UNION ALL SELECT t,c,r.id,r.currency,r.cap,r.actual,true,p.fresh_until,s,'BUDGET',jsonb_build_array(p.source_system,r.cost_center,r.account,r.period_start,r.period_end)::text FROM finance.budget_ref r WHERE r.tenant_id=t AND r.company_id=c AND r.snapshot_id=s AND day>=r.period_start AND day<r.period_end;
 GET DIAGNOSTICS n=ROW_COUNT;
 IF n=0 THEN RAISE EXCEPTION 'no effective resources' USING ERRCODE='23514'; END IF;
 UPDATE finance.reference_snapshot SET state='APPROVED',checker_id=a WHERE tenant_id=t AND company_id=c AND id=s;
END $$;
CREATE FUNCTION finance.prototype_revoke_reference(t uuid,c uuid,s uuid,a uuid) RETURNS void
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE p finance.reference_snapshot%ROWTYPE;
BEGIN
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reference/v1/'||t::text||'/'||c::text,0));
 SELECT * INTO p FROM finance.reference_snapshot WHERE tenant_id=t AND company_id=c AND id=s FOR UPDATE;
 IF NOT FOUND OR p.state<>'APPROVED' OR a IS DISTINCT FROM p.checker_id OR NOT EXISTS(SELECT 1 FROM finance.membership WHERE tenant_id=t AND company_id=c AND actor_id=a AND role='FINANCE_APPROVER' AND active AND valid_from<=clock_timestamp() AND (valid_until IS NULL OR valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'invalid revoke' USING ERRCODE='23514'; END IF;
 -- Lock in same order as reserve; existing committed holds are NOT released.
 PERFORM 1 FROM finance.financial_resource WHERE tenant_id=t AND company_id=c AND snapshot_id=s ORDER BY id FOR UPDATE;
 UPDATE finance.financial_resource SET coverage_complete=false WHERE tenant_id=t AND company_id=c AND snapshot_id=s;
 UPDATE finance.reference_snapshot SET state='REVOKED' WHERE tenant_id=t AND company_id=c AND id=s;
END $$;
REVOKE ALL ON FUNCTION finance.prototype_approve_reference(uuid,uuid,uuid,uuid,date),finance.prototype_revoke_reference(uuid,uuid,uuid,uuid) FROM PUBLIC;
