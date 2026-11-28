-- Explicit release of a locally held, unsealed reservation. Never release
-- a committed, consumed, handed-off or sealed financial obligation.
ALTER TABLE finance.reservation ADD COLUMN release_actor_id uuid,ADD COLUMN release_key text,
 ADD COLUMN release_request_hash bytea,ADD COLUMN release_version bigint,ADD COLUMN release_reason text,
 ADD CHECK((release_key IS NULL AND release_actor_id IS NULL AND release_request_hash IS NULL AND release_version IS NULL AND release_reason IS NULL)
 OR (release_key IS NOT NULL AND release_actor_id IS NOT NULL AND release_request_hash IS NOT NULL AND octet_length(release_request_hash)=32 AND release_version IS NOT NULL AND release_reason IS NOT NULL AND state='RELEASED'));
CREATE UNIQUE INDEX reservation_release_replay ON finance.reservation(tenant_id,company_id,release_actor_id,release_key) WHERE release_key IS NOT NULL;
GRANT UPDATE(state,remaining,release_actor_id,release_key,release_request_hash,release_version,release_reason) ON finance.reservation TO finance_workflow_writer;
GRANT SELECT,INSERT ON finance.allocation_effect TO finance_workflow_writer;
CREATE POLICY release_effect_read ON finance.allocation_effect FOR SELECT TO finance_workflow_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY release_effect_insert ON finance.allocation_effect FOR INSERT TO finance_workflow_writer WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND kind='RELEASE' AND reverses IS NULL);
DO $$ DECLARE expr text; BEGIN
 SELECT pg_get_expr(conbin,conrelid) INTO STRICT expr FROM pg_constraint WHERE conrelid='finance.audit_event'::regclass AND conname='audit_event_action_check';
 EXECUTE 'ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check';
 EXECUTE format('ALTER TABLE finance.audit_event ADD CONSTRAINT audit_event_action_check CHECK ((%s) OR action=%L)',expr,'FINANCE_RESERVATION_RELEASED');
END $$;
CREATE FUNCTION finance.reservation_release_immutable() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 IF OLD.release_key IS NOT NULL AND (OLD.release_actor_id,OLD.release_key,OLD.release_request_hash,OLD.release_version,OLD.release_reason)
 IS DISTINCT FROM (NEW.release_actor_id,NEW.release_key,NEW.release_request_hash,NEW.release_version,NEW.release_reason)
 THEN RAISE EXCEPTION 'immutable release receipt' USING ERRCODE='23514'; END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER reservation_release_immutable BEFORE UPDATE ON finance.reservation FOR EACH ROW EXECUTE FUNCTION finance.reservation_release_immutable();
CREATE FUNCTION finance.runtime_release_unsealed_reservation(t uuid,c uuid,a uuid,d uuid,v bigint,i uuid,why text,idem text,cmd uuid,corr uuid)
RETURNS TABLE(id uuid,dossier_version bigint) LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ds finance.dossier%ROWTYPE; hold finance.reservation%ROWTYPE; rev uuid; prior finance.reservation%ROWTYPE; digest bytea; k uuid;
BEGIN
 IF t IS NULL OR c IS NULL OR a IS NULL OR d IS NULL OR i IS NULL OR v IS NULL OR v<0 OR cmd IS NULL OR corr IS NULL
 OR (t,c,a) IS DISTINCT FROM (finance.scope_tenant(),finance.scope_company(),finance.scope_actor())
 OR why IS NULL OR btrim(why)='' OR octet_length(why)>2048 OR why ~ '[[:cntrl:]]' OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$'
 THEN RAISE EXCEPTION 'invalid release command' USING ERRCODE='22023'; END IF;
 IF NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a) AND m.role='FINANCE_APPROVER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'finance approver required' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reservation.release/v1/'||t::text||'/'||c::text||'/'||a::text||'/'||idem,0));
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO ds FROM finance.dossier WHERE (tenant_id,company_id,id)=(t,c,d) FOR UPDATE;
 IF NOT FOUND OR ds.owner_id=a THEN RAISE EXCEPTION 'independent finance approver required' USING ERRCODE='42501'; END IF;
 digest:=sha256(convert_to(jsonb_build_array(d,v,i,why)::text,'UTF8'));
 SELECT * INTO prior FROM finance.reservation x WHERE (x.tenant_id,x.company_id,x.release_actor_id,x.release_key)=(t,c,a,idem);
 IF FOUND THEN
  IF prior.id<>i OR prior.release_request_hash<>digest THEN RAISE EXCEPTION 'release replay conflict' USING ERRCODE='23505'; END IF;
  RETURN QUERY SELECT prior.id,prior.release_version; RETURN;
 END IF;
 IF ds.version<>v OR ds.status NOT IN ('DRAFT','NEEDS_INPUT','REVIEWABLE') THEN RAISE EXCEPTION 'stale or sealed dossier' USING ERRCODE='23514'; END IF;
 SELECT dr.id INTO STRICT rev FROM finance.dossier_revision dr WHERE (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number)=(t,c,d,ds.revision);
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.reservation/v1/'||t::text||'/'||c::text||'/'||i::text,0));
 SELECT * INTO hold FROM finance.reservation x WHERE (x.tenant_id,x.company_id,x.id,x.revision_id)=(t,c,i,rev) FOR UPDATE;
 IF NOT FOUND OR hold.binding_policy_id IS NULL OR hold.state<>'RESERVED' OR hold.remaining<>hold.amount
 OR EXISTS(SELECT 1 FROM finance.dossier_snapshot s WHERE (s.tenant_id,s.company_id,s.reservation_id)=(t,c,i))
 OR EXISTS(SELECT 1 FROM finance.allocation_effect e WHERE (e.tenant_id,e.company_id,e.reservation_id)=(t,c,i))
 THEN RAISE EXCEPTION 'release requires unsealed unused hold' USING ERRCODE='23514'; END IF;
 FOR k IN SELECT z FROM unnest(hold.resource_ids) z ORDER BY z LOOP
  PERFORM 1 FROM finance.financial_resource fr WHERE (fr.tenant_id,fr.company_id,fr.id)=(t,c,k) FOR UPDATE;
  IF NOT FOUND THEN RAISE EXCEPTION 'resource absent' USING ERRCODE='23514'; END IF;
 END LOOP;
 UPDATE finance.reservation_line l SET line_remaining=0 WHERE (l.tenant_id,l.company_id,l.reservation_id)=(t,c,i) AND l.line_remaining IS NOT NULL;
 UPDATE finance.reservation x SET state='RELEASED',remaining=0,release_actor_id=a,release_key=idem,
 release_request_hash=digest,release_version=v+1,release_reason=why WHERE (x.tenant_id,x.company_id,x.id)=(t,c,i);
 INSERT INTO finance.allocation_effect(tenant_id,company_id,id,reservation_id,revision_id,kind,amount,source_digest,reverses)
 VALUES(t,c,cmd,i,rev,'RELEASE',hold.remaining,sha256(convert_to(jsonb_build_array(encode(hold.binding_material_hash,'hex'),digest)::text,'UTF8')),NULL);
 UPDATE finance.dossier x SET version=v+1,status='DRAFT',updated_at=GREATEST(updated_at,clock_timestamp()) WHERE (x.tenant_id,x.company_id,x.id)=(t,c,d);
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES(t,c,cmd,a,cmd,corr,i,'FINANCE_RESERVATION_RELEASED');
 RETURN QUERY SELECT i,v+1;
END $$;
ALTER FUNCTION finance.runtime_release_unsealed_reservation(uuid,uuid,uuid,uuid,bigint,uuid,text,text,uuid,uuid) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.runtime_release_unsealed_reservation(uuid,uuid,uuid,uuid,bigint,uuid,text,text,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_release_unsealed_reservation(uuid,uuid,uuid,uuid,bigint,uuid,text,text,uuid,uuid) TO finance_runtime;
