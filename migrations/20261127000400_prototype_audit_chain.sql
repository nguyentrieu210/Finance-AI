-- Synthetic-only audit chain. A dedicated NOLOGIN trigger role owns no tables.
CREATE ROLE finance_audit_chain_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_audit_chain_writer;
CREATE TABLE finance.audit_chain_head (
 tenant_id uuid PRIMARY KEY REFERENCES finance.tenant(id),
 sequence bigint NOT NULL CHECK(sequence>=0),
 digest bytea NOT NULL CHECK(octet_length(digest)=32)
);
ALTER TABLE finance.audit_chain_head ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.audit_chain_head FORCE ROW LEVEL SECURITY;
CREATE POLICY chain_scope ON finance.audit_chain_head TO finance_audit_chain_writer
 USING(tenant_id=finance.scope_tenant()) WITH CHECK(tenant_id=finance.scope_tenant());
GRANT SELECT,INSERT ON finance.audit_chain_head TO finance_audit_chain_writer;
GRANT UPDATE(sequence,digest) ON finance.audit_chain_head TO finance_audit_chain_writer;
ALTER TABLE finance.audit_event ADD COLUMN sequence bigint,
 ADD COLUMN previous_digest bytea, ADD COLUMN digest bytea;
-- Canonical v1: domain, previous digest, tenant/company/event/actor/command/correlation/resource UUID bytes,
-- signed big-endian sequence and Unix microseconds, then UTF-8 action (last field).
CREATE FUNCTION finance.audit_digest_v1(previous bytea,t uuid,c uuid,e uuid,a uuid,cmd uuid,corr uuid,r uuid,seq bigint,ts timestamptz,act text)
RETURNS bytea LANGUAGE sql IMMUTABLE STRICT SET search_path=pg_catalog AS $$
 SELECT sha256(convert_to('finance.audit/v1','UTF8') || previous ||
 uuid_send(t)||uuid_send(c)||uuid_send(e)||uuid_send(a)||uuid_send(cmd)||uuid_send(corr)||uuid_send(r)||
 int8send(seq)||int8send((extract(epoch FROM ts)*1000000)::bigint)||convert_to(act,'UTF8'))
$$;
REVOKE ALL ON FUNCTION finance.audit_digest_v1(bytea,uuid,uuid,uuid,uuid,uuid,uuid,uuid,bigint,timestamptz,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.audit_digest_v1(bytea,uuid,uuid,uuid,uuid,uuid,uuid,uuid,bigint,timestamptz,text) TO finance_audit_chain_writer;
-- Preserve existing history in deterministic order. This establishes a migration-time genesis,
-- not proof that historical rows were untampered before this migration.
DO $$
DECLARE e finance.audit_event%ROWTYPE; s bigint; prev bytea; h bytea;
BEGIN
 FOR e IN SELECT * FROM finance.audit_event ORDER BY tenant_id,occurred_at,company_id,id LOOP
  INSERT INTO finance.audit_chain_head VALUES(e.tenant_id,0,decode(repeat('00',32),'hex')) ON CONFLICT DO NOTHING;
  SELECT sequence,digest INTO s,prev FROM finance.audit_chain_head WHERE tenant_id=e.tenant_id;
  s:=s+1;
  h:=finance.audit_digest_v1(prev,e.tenant_id,e.company_id,e.id,e.actor_id,e.command_id,e.correlation_id,e.resource_id,s,e.occurred_at,e.action);
  UPDATE finance.audit_event SET sequence=s,previous_digest=prev,digest=h WHERE (tenant_id,company_id,id)=(e.tenant_id,e.company_id,e.id);
  UPDATE finance.audit_chain_head SET sequence=s,digest=h WHERE tenant_id=e.tenant_id;
 END LOOP;
END $$;
ALTER TABLE finance.audit_event ALTER COLUMN sequence SET NOT NULL,
 ALTER COLUMN previous_digest SET NOT NULL, ALTER COLUMN digest SET NOT NULL,
 ADD CONSTRAINT audit_sequence_positive CHECK(sequence>0),
 ADD CONSTRAINT audit_digest_lengths CHECK(octet_length(previous_digest)=32 AND octet_length(digest)=32),
 ADD CONSTRAINT audit_tenant_sequence UNIQUE(tenant_id,sequence);
ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN ('ORG_UNIT_CREATED','AUDIT_READ'));
CREATE FUNCTION finance.audit_chain_append() RETURNS trigger LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE s bigint; prev bytea;
BEGIN
 IF NEW.tenant_id IS DISTINCT FROM finance.scope_tenant()
 OR NEW.company_id IS DISTINCT FROM finance.scope_company()
 OR NEW.actor_id IS DISTINCT FROM finance.scope_actor() THEN
  RAISE EXCEPTION 'audit scope mismatch' USING ERRCODE='42501';
 END IF;
 IF NEW.sequence IS NOT NULL OR NEW.previous_digest IS NOT NULL OR NEW.digest IS NOT NULL THEN
  RAISE EXCEPTION 'caller-supplied audit chain metadata forbidden' USING ERRCODE='22023';
 END IF;
 INSERT INTO finance.audit_chain_head VALUES(NEW.tenant_id,0,decode(repeat('00',32),'hex')) ON CONFLICT DO NOTHING;
 SELECT sequence,digest INTO STRICT s,prev FROM finance.audit_chain_head WHERE tenant_id=NEW.tenant_id FOR UPDATE;
 NEW.sequence:=s+1;
 NEW.previous_digest:=prev;
 NEW.occurred_at:=clock_timestamp();
 NEW.digest:=finance.audit_digest_v1(prev,NEW.tenant_id,NEW.company_id,NEW.id,NEW.actor_id,NEW.command_id,NEW.correlation_id,NEW.resource_id,NEW.sequence,NEW.occurred_at,NEW.action);
 UPDATE finance.audit_chain_head SET sequence=NEW.sequence,digest=NEW.digest WHERE tenant_id=NEW.tenant_id;
 RETURN NEW;
END $$;
ALTER FUNCTION finance.audit_chain_append() OWNER TO finance_audit_chain_writer;
REVOKE ALL ON FUNCTION finance.audit_chain_append() FROM PUBLIC;
CREATE TRIGGER audit_chain_before_insert BEFORE INSERT ON finance.audit_event FOR EACH ROW EXECUTE FUNCTION finance.audit_chain_append();
CREATE INDEX audit_company_sequence ON finance.audit_event(tenant_id,company_id,sequence);
CREATE POLICY audit_scoped_read ON finance.audit_event FOR SELECT TO finance_runtime USING(
 tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND EXISTS(
 SELECT 1 FROM finance.membership m JOIN finance.identity_subject i ON (i.tenant_id,i.id)=(m.tenant_id,m.actor_id)
 JOIN finance.company c ON (c.tenant_id,c.id)=(m.tenant_id,m.company_id)
 JOIN finance.tenant t ON t.id=m.tenant_id
 WHERE m.tenant_id=finance.scope_tenant() AND m.company_id=finance.scope_company() AND m.actor_id=finance.scope_actor()
 AND m.role='AUDITOR' AND m.active AND i.active AND c.active AND t.active
 AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())
 ));
GRANT SELECT ON finance.audit_event TO finance_runtime;
-- Runtime has no direct chain head access and no UPDATE/DELETE on audit events.
