-- E16 synthetic governance kernel; no real business policy activation or runtime privileges.
CREATE TABLE finance.policy_version (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 dossier_kind text NOT NULL CHECK(dossier_kind IN ('PAYMENT','ADVANCE','SETTLEMENT')),
 author_id uuid NOT NULL, checker_id uuid, state text NOT NULL DEFAULT 'DRAFT'
  CHECK(state IN ('DRAFT','IN_REVIEW','PUBLISHED','RETIRED','REVOKED')),
 effective_from date NOT NULL, effective_until date NOT NULL,
 source_digest bytea NOT NULL CHECK(octet_length(source_digest)=32),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(), published_at timestamptz,
 PRIMARY KEY(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id) REFERENCES finance.company(tenant_id,id),
 FOREIGN KEY(tenant_id,author_id) REFERENCES finance.identity_subject(tenant_id,id),
 FOREIGN KEY(tenant_id,checker_id) REFERENCES finance.identity_subject(tenant_id,id),
 CHECK(effective_until>effective_from),
 CHECK(checker_id IS NULL OR checker_id<>author_id),
 CHECK((state IN ('DRAFT','IN_REVIEW') AND checker_id IS NULL AND published_at IS NULL)
 OR (state IN ('PUBLISHED','RETIRED','REVOKED') AND checker_id IS NOT NULL AND published_at IS NOT NULL))
);
CREATE INDEX policy_effective ON finance.policy_version(tenant_id,company_id,dossier_kind,effective_from,effective_until) WHERE state='PUBLISHED';
CREATE TABLE finance.policy_clause (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, policy_id uuid NOT NULL, id uuid NOT NULL,
 clause_key text NOT NULL CHECK(clause_key ~ '^[A-Za-z0-9._-]{1,64}$'),
 excerpt text NOT NULL CHECK(octet_length(excerpt) BETWEEN 1 AND 8192),
 source_digest bytea NOT NULL CHECK(octet_length(source_digest)=32),
 source_page integer NOT NULL CHECK(source_page BETWEEN 1 AND 200),
 PRIMARY KEY(tenant_id,company_id,policy_id,id), UNIQUE(tenant_id,company_id,policy_id,clause_key),
 FOREIGN KEY(tenant_id,company_id,policy_id) REFERENCES finance.policy_version(tenant_id,company_id,id)
);
CREATE TABLE finance.checklist_item (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, policy_id uuid NOT NULL, id uuid NOT NULL, clause_id uuid NOT NULL,
 document_kind text NOT NULL CHECK(document_kind ~ '^[A-Z_]{1,64}$'),
 mandatory boolean NOT NULL,
 PRIMARY KEY(tenant_id,company_id,policy_id,id), UNIQUE(tenant_id,company_id,policy_id,document_kind),
 FOREIGN KEY(tenant_id,company_id,policy_id,clause_id) REFERENCES finance.policy_clause(tenant_id,company_id,policy_id,id)
);
ALTER TABLE finance.policy_version ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.policy_version FORCE ROW LEVEL SECURITY;
ALTER TABLE finance.policy_clause ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.policy_clause FORCE ROW LEVEL SECURITY;
ALTER TABLE finance.checklist_item ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.checklist_item FORCE ROW LEVEL SECURITY;
-- All mutations serialized by scope/kind; no direct runtime access.
CREATE FUNCTION finance.prototype_policy_lock(t uuid,c uuid,k text) RETURNS void
LANGUAGE sql VOLATILE SET search_path=pg_catalog AS $$
 SELECT pg_advisory_xact_lock(hashtextextended('finance.policy/v1/'||t::text||'/'||c::text||'/'||k,0))
$$;
CREATE FUNCTION finance.prototype_policy_child_guard() RETURNS trigger
LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE p finance.policy_version%ROWTYPE; t uuid; c uuid; i uuid;
BEGIN
 IF TG_OP='DELETE' THEN t:=OLD.tenant_id;c:=OLD.company_id;i:=OLD.policy_id;
 ELSE t:=NEW.tenant_id;c:=NEW.company_id;i:=NEW.policy_id; END IF;
 IF TG_OP='UPDATE' AND (NEW.tenant_id,NEW.company_id,NEW.policy_id,NEW.id) IS DISTINCT FROM
 (OLD.tenant_id,OLD.company_id,OLD.policy_id,OLD.id) THEN RAISE EXCEPTION 'immutable child binding' USING ERRCODE='23514'; END IF;
 SELECT * INTO p FROM finance.policy_version WHERE tenant_id=t AND company_id=c AND id=i FOR UPDATE;
 IF NOT FOUND OR p.state<>'DRAFT' THEN RAISE EXCEPTION 'policy content frozen' USING ERRCODE='23514'; END IF;
 IF TG_TABLE_NAME='policy_clause' AND TG_OP<>'DELETE' THEN
  IF NEW.source_digest<>p.source_digest THEN RAISE EXCEPTION 'clause source mismatch' USING ERRCODE='23514'; END IF;
 END IF;
 IF TG_OP='DELETE' THEN RETURN OLD; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER clause_guard BEFORE INSERT OR UPDATE OR DELETE ON finance.policy_clause FOR EACH ROW EXECUTE FUNCTION finance.prototype_policy_child_guard();
CREATE TRIGGER checklist_guard BEFORE INSERT OR UPDATE OR DELETE ON finance.checklist_item FOR EACH ROW EXECUTE FUNCTION finance.prototype_policy_child_guard();

CREATE FUNCTION finance.prototype_policy_transition(t uuid,c uuid,i uuid,actor uuid,target text)
RETURNS void LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE p finance.policy_version%ROWTYPE; k text;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' OR actor IS NULL OR target IS NULL THEN
  RAISE EXCEPTION 'invalid policy transition' USING ERRCODE='23514'; END IF;
 SELECT dossier_kind INTO k FROM finance.policy_version WHERE tenant_id=t AND company_id=c AND id=i;
 IF NOT FOUND THEN RAISE EXCEPTION 'unknown policy' USING ERRCODE='23514'; END IF;
 PERFORM finance.prototype_policy_lock(t,c,k);
 SELECT * INTO p FROM finance.policy_version WHERE tenant_id=t AND company_id=c AND id=i FOR UPDATE;
 IF NOT FOUND OR p.dossier_kind<>k THEN RAISE EXCEPTION 'changed policy' USING ERRCODE='23514'; END IF;
 IF target='IN_REVIEW' AND p.state='DRAFT' AND actor=p.author_id THEN
  IF NOT EXISTS(SELECT 1 FROM finance.policy_clause WHERE tenant_id=t AND company_id=c AND policy_id=i)
  OR NOT EXISTS(SELECT 1 FROM finance.checklist_item WHERE tenant_id=t AND company_id=c AND policy_id=i AND mandatory)
  THEN RAISE EXCEPTION 'unmapped policy' USING ERRCODE='23514'; END IF;
  UPDATE finance.policy_version SET state=target WHERE tenant_id=t AND company_id=c AND id=i;
 ELSIF target='PUBLISHED' AND p.state='IN_REVIEW' AND actor<>p.author_id THEN
  IF NOT EXISTS(SELECT 1 FROM finance.membership WHERE tenant_id=t AND company_id=c AND actor_id=actor
    AND role='FINANCE_APPROVER' AND active AND valid_from<=clock_timestamp() AND (valid_until IS NULL OR valid_until>clock_timestamp()))
  THEN RAISE EXCEPTION 'independent checker required' USING ERRCODE='23514'; END IF;
  IF EXISTS(SELECT 1 FROM finance.policy_version WHERE tenant_id=t AND company_id=c AND dossier_kind=k AND state='PUBLISHED'
    AND effective_from<p.effective_until AND p.effective_from<effective_until)
  THEN RAISE EXCEPTION 'overlapping effective policy' USING ERRCODE='23514'; END IF;
  UPDATE finance.policy_version SET state=target,checker_id=actor,published_at=clock_timestamp() WHERE tenant_id=t AND company_id=c AND id=i;
 ELSIF target IN ('RETIRED','REVOKED') AND p.state='PUBLISHED' AND actor=p.checker_id THEN
  IF NOT EXISTS(SELECT 1 FROM finance.membership WHERE tenant_id=t AND company_id=c AND actor_id=actor
    AND role='FINANCE_APPROVER' AND active AND valid_from<=clock_timestamp() AND (valid_until IS NULL OR valid_until>clock_timestamp()))
  THEN RAISE EXCEPTION 'checker no longer active' USING ERRCODE='23514'; END IF;
  UPDATE finance.policy_version SET state=target WHERE tenant_id=t AND company_id=c AND id=i;
 ELSE RAISE EXCEPTION 'invalid policy transition' USING ERRCODE='23514'; END IF;
END $$;
CREATE FUNCTION finance.prototype_effective_policy(t uuid,c uuid,k text,day date)
RETURNS uuid LANGUAGE plpgsql STABLE SET search_path=pg_catalog AS $$
DECLARE result uuid;
BEGIN
 SELECT id INTO STRICT result FROM finance.policy_version
 WHERE tenant_id=t AND company_id=c AND dossier_kind=k AND state='PUBLISHED'
  AND effective_from<=day AND day<effective_until;
 RETURN result;
EXCEPTION WHEN no_data_found THEN RETURN NULL;
 WHEN too_many_rows THEN RAISE EXCEPTION 'ambiguous effective policy' USING ERRCODE='23514';
END $$;
REVOKE ALL ON FUNCTION finance.prototype_policy_lock(uuid,uuid,text) FROM PUBLIC;
REVOKE ALL ON FUNCTION finance.prototype_policy_transition(uuid,uuid,uuid,uuid,text) FROM PUBLIC;
REVOKE ALL ON FUNCTION finance.prototype_effective_policy(uuid,uuid,text,date) FROM PUBLIC;
-- No application mutation privileges; fixture owner can bypass all prototype policy guarantees.
