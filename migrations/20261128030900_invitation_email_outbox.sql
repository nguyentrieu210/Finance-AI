-- Durable Resend delivery for explicitly issued staff invitations.
CREATE ROLE finance_invite_dispatcher NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_invite_dispatcher;
CREATE TABLE finance.staff_invitation_delivery (
 tenant_id uuid NOT NULL,
 company_id uuid NOT NULL,
 delivery_id uuid NOT NULL DEFAULT gen_random_uuid(),
 invitation_id uuid NOT NULL,
 status text NOT NULL DEFAULT 'QUEUED' CHECK(status IN ('QUEUED','SENDING','SENT','FAILED','CANCELLED')),
 attempts integer NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 8),
 next_attempt_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 locked_by uuid,
 locked_until timestamptz,
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 sent_at timestamptz,
 last_error_code text CHECK(last_error_code IS NULL OR last_error_code IN ('NETWORK','PROVIDER_4XX','PROVIDER_5XX','PROVIDER_429')),
 PRIMARY KEY(tenant_id,company_id,delivery_id),
 UNIQUE(tenant_id,company_id,invitation_id),
 FOREIGN KEY(tenant_id,company_id,invitation_id) REFERENCES finance.staff_invitation(tenant_id,company_id,id),
 CHECK((status='SENDING')=(locked_by IS NOT NULL AND locked_until IS NOT NULL)),
 CHECK((status='SENT')=(sent_at IS NOT NULL))
);
ALTER TABLE finance.staff_invitation_delivery ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.staff_invitation_delivery FORCE ROW LEVEL SECURITY;
GRANT INSERT,SELECT,UPDATE(status,attempts,next_attempt_at,locked_by,locked_until,sent_at,last_error_code) ON finance.staff_invitation_delivery TO finance_invite_dispatcher;
GRANT SELECT ON finance.staff_invitation_delivery TO finance_membership_writer;
GRANT SELECT,UPDATE(status,attempts,next_attempt_at,locked_by,locked_until,sent_at,last_error_code)
 ON finance.staff_invitation_delivery TO finance_invite_dispatcher;
GRANT SELECT,UPDATE(status,expires_at,version) ON finance.staff_invitation TO finance_invite_dispatcher;
GRANT SELECT ON finance.company TO finance_invite_dispatcher;
GRANT EXECUTE ON FUNCTION finance.membership_admin_allowed(uuid,uuid,uuid) TO finance_invite_dispatcher;
CREATE POLICY delivery_admin_scope ON finance.staff_invitation_delivery FOR SELECT TO finance_membership_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY delivery_dispatcher_all ON finance.staff_invitation_delivery TO finance_invite_dispatcher
 USING(true) WITH CHECK(true);
CREATE POLICY invitation_dispatcher_all ON finance.staff_invitation TO finance_invite_dispatcher USING(true) WITH CHECK(true);
CREATE POLICY company_dispatcher_read ON finance.company FOR SELECT TO finance_invite_dispatcher USING(true);

CREATE OR REPLACE FUNCTION finance.staff_invitation_issue(t uuid,c uuid,a uuid,rid uuid,expected bigint,issue boolean)
RETURNS bigint LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v bigint;
BEGIN
 IF NOT coalesce(finance.membership_admin_allowed(t,c,a),false) OR issue IS NULL OR expected IS NULL THEN RAISE EXCEPTION 'forbidden' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('staff-admin/'||t::text||'/'||c::text,0));
 UPDATE finance.staff_invitation SET status=CASE WHEN issue THEN 'ISSUED' ELSE 'CANCELLED' END,
  expires_at=CASE WHEN issue THEN clock_timestamp()+interval '7 days' ELSE expires_at END,version=version+1
 WHERE tenant_id=t AND company_id=c AND id=rid AND version=expected
  AND (CASE WHEN issue THEN status='DRAFT' ELSE status='ISSUED' END)
 RETURNING version INTO v;
 IF v IS NULL THEN RAISE EXCEPTION 'stale invitation' USING ERRCODE='23514'; END IF;
 IF issue THEN
  INSERT INTO finance.staff_invitation_delivery(tenant_id,company_id,invitation_id)
  VALUES(t,c,rid) ON CONFLICT(tenant_id,company_id,invitation_id) DO NOTHING;
 ELSE
  UPDATE finance.staff_invitation_delivery SET status='CANCELLED',locked_by=NULL,locked_until=NULL
   WHERE tenant_id=t AND company_id=c AND invitation_id=rid AND status IN ('QUEUED','SENDING');
 END IF;
 RETURN v;
END $$;
ALTER FUNCTION finance.staff_invitation_issue(uuid,uuid,uuid,uuid,bigint,boolean) OWNER TO finance_invite_dispatcher;
REVOKE ALL ON FUNCTION finance.staff_invitation_issue(uuid,uuid,uuid,uuid,bigint,boolean) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.staff_invitation_issue(uuid,uuid,uuid,uuid,bigint,boolean) TO finance_runtime;

CREATE OR REPLACE FUNCTION finance.staff_admin_list(t uuid,c uuid,a uuid,kind text,after_id uuid,n integer)
RETURNS SETOF jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE tbl text;
BEGIN
 IF NOT coalesce(finance.membership_admin_allowed(t,c,a),false) OR n IS NULL OR n NOT BETWEEN 1 AND 100 OR after_id IS NULL THEN RAISE EXCEPTION 'forbidden' USING ERRCODE='42501'; END IF;
 tbl:=CASE kind WHEN 'positions' THEN 'staff_position' WHEN 'departments' THEN 'staff_department' WHEN 'invitations' THEN 'staff_invitation' WHEN 'roles' THEN 'staff_role' WHEN 'assignments' THEN 'staff_role_assignment' END;
 IF tbl IS NULL THEN RAISE EXCEPTION 'invalid kind' USING ERRCODE='22023'; END IF;
 IF kind='roles' THEN
  RETURN QUERY SELECT (to_jsonb(r)-'tenant_id'-'company_id')||jsonb_build_object('capabilities',v.capabilities)
    FROM finance.staff_role r JOIN finance.staff_role_revision v ON (v.tenant_id,v.company_id,v.role_id,v.version)=(r.tenant_id,r.company_id,r.id,r.version)
    WHERE r.tenant_id=t AND r.company_id=c AND r.id>after_id ORDER BY r.id LIMIT n;
 ELSIF kind='invitations' THEN
  RETURN QUERY SELECT (to_jsonb(x)-'tenant_id'-'company_id')||jsonb_build_object('delivery_status',coalesce(d.status,'NOT_QUEUED'))
    FROM finance.staff_invitation x LEFT JOIN finance.staff_invitation_delivery d
      ON (d.tenant_id,d.company_id,d.invitation_id)=(x.tenant_id,x.company_id,x.id)
    WHERE x.tenant_id=t AND x.company_id=c AND x.id>after_id ORDER BY x.id LIMIT n;
 ELSE
  RETURN QUERY EXECUTE format('SELECT to_jsonb(x)-''tenant_id''-''company_id'' FROM finance.%I x WHERE tenant_id=$1 AND company_id=$2 AND id>$3 ORDER BY id LIMIT $4',tbl) USING t,c,after_id,n;
 END IF;
END $$;
ALTER FUNCTION finance.staff_admin_list(uuid,uuid,uuid,text,uuid,integer) OWNER TO finance_membership_writer;
REVOKE ALL ON FUNCTION finance.staff_admin_list(uuid,uuid,uuid,text,uuid,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.staff_admin_list(uuid,uuid,uuid,text,uuid,integer) TO finance_runtime;

CREATE FUNCTION finance.staff_invitation_delivery_claim(worker uuid,n integer)
RETURNS TABLE(delivery_id uuid,tenant_id uuid,company_id uuid,invitation_id uuid,email text,company_name text)
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF worker IS NULL OR worker='00000000-0000-0000-0000-000000000000' OR n IS NULL OR n NOT BETWEEN 1 AND 20 THEN RAISE EXCEPTION 'invalid claim' USING ERRCODE='22023'; END IF;
 RETURN QUERY
 WITH candidate AS (
  SELECT d.tenant_id,d.company_id,d.delivery_id
  FROM finance.staff_invitation_delivery d
  JOIN finance.staff_invitation i ON (i.tenant_id,i.company_id,i.id)=(d.tenant_id,d.company_id,d.invitation_id)
  WHERE i.status='ISSUED' AND i.expires_at>clock_timestamp() AND d.attempts<8
   AND ((d.status='QUEUED' AND d.next_attempt_at<=clock_timestamp()) OR (d.status='SENDING' AND d.locked_until<clock_timestamp()))
  ORDER BY d.next_attempt_at,d.created_at,d.delivery_id
  LIMIT n FOR UPDATE SKIP LOCKED
 ), claimed AS (
  UPDATE finance.staff_invitation_delivery d SET status='SENDING',attempts=d.attempts+1,locked_by=worker,
   locked_until=clock_timestamp()+interval '2 minutes',last_error_code=NULL
  FROM candidate x WHERE (d.tenant_id,d.company_id,d.delivery_id)=(x.tenant_id,x.company_id,x.delivery_id)
  RETURNING d.delivery_id,d.tenant_id,d.company_id,d.invitation_id
 )
 SELECT q.delivery_id,q.tenant_id,q.company_id,q.invitation_id,i.email,c.name
 FROM claimed q JOIN finance.staff_invitation i ON (i.tenant_id,i.company_id,i.id)=(q.tenant_id,q.company_id,q.invitation_id)
 JOIN finance.company c ON (c.tenant_id,c.id)=(q.tenant_id,q.company_id)
 WHERE i.status='ISSUED' AND i.expires_at>clock_timestamp();
END $$;
ALTER FUNCTION finance.staff_invitation_delivery_claim(uuid,integer) OWNER TO finance_invite_dispatcher;
REVOKE ALL ON FUNCTION finance.staff_invitation_delivery_claim(uuid,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.staff_invitation_delivery_claim(uuid,integer) TO finance_runtime;

CREATE FUNCTION finance.staff_invitation_delivery_complete(worker uuid,rid uuid,sent boolean,error_code text)
RETURNS boolean LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE changed integer;
BEGIN
 IF worker IS NULL OR rid IS NULL OR sent IS NULL OR (NOT sent AND error_code NOT IN ('NETWORK','PROVIDER_4XX','PROVIDER_5XX','PROVIDER_429')) THEN RAISE EXCEPTION 'invalid completion' USING ERRCODE='22023'; END IF;
 UPDATE finance.staff_invitation_delivery SET
  status=CASE WHEN sent THEN 'SENT' WHEN error_code='PROVIDER_4XX' OR attempts>=8 THEN 'FAILED' ELSE 'QUEUED' END,
  next_attempt_at=CASE WHEN sent OR error_code='PROVIDER_4XX' OR attempts>=8 THEN next_attempt_at
    ELSE clock_timestamp()+make_interval(secs=>LEAST(3600,15*(2^LEAST(attempts,8))::integer)) END,
  locked_by=NULL,locked_until=NULL,sent_at=CASE WHEN sent THEN clock_timestamp() ELSE NULL END,
  last_error_code=CASE WHEN sent THEN NULL ELSE error_code END
 WHERE delivery_id=rid AND locked_by=worker AND status='SENDING';
 GET DIAGNOSTICS changed=ROW_COUNT;
 RETURN changed=1;
END $$;
ALTER FUNCTION finance.staff_invitation_delivery_complete(uuid,uuid,boolean,text) OWNER TO finance_invite_dispatcher;
REVOKE ALL ON FUNCTION finance.staff_invitation_delivery_complete(uuid,uuid,boolean,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.staff_invitation_delivery_complete(uuid,uuid,boolean,text) TO finance_runtime;
