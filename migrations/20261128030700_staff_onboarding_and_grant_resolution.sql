-- Candidate: explicit invitation issuance and verified self-claim, followed by admin approval.
-- No authorization changes to existing dossier RLS or financial commands.
ALTER TABLE finance.staff_invitation
 DROP CONSTRAINT staff_invitation_status_check,
 ADD CONSTRAINT staff_invitation_status_check CHECK(status IN ('DRAFT','ISSUED','ACCEPTED','CANCELLED')),
 ADD COLUMN expires_at timestamptz;
DROP INDEX finance.staff_invite_open;
CREATE UNIQUE INDEX staff_invite_open ON finance.staff_invitation(tenant_id,company_id,lower(email)) WHERE status IN ('DRAFT','ISSUED');
CREATE TABLE finance.staff_onboarding_request (
 tenant_id uuid NOT NULL, company_id uuid NOT NULL, id uuid NOT NULL,
 invitation_id uuid NOT NULL, issuer text NOT NULL CHECK(length(issuer) BETWEEN 1 AND 512),
 subject text NOT NULL CHECK(length(subject) BETWEEN 1 AND 256),
 email text NOT NULL CHECK(length(email) BETWEEN 3 AND 320),
 display_name text CHECK(display_name IS NULL OR length(display_name) BETWEEN 1 AND 200),
 profile_issued_at bigint NOT NULL CHECK(profile_issued_at>=0),
 status text NOT NULL DEFAULT 'PENDING' CHECK(status IN ('PENDING','APPROVED','REJECTED')),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 decided_at timestamptz, decided_by uuid, actor_id uuid,
 version bigint NOT NULL DEFAULT 0 CHECK(version BETWEEN 0 AND 9007199254740991),
 PRIMARY KEY(tenant_id,company_id,id),
 UNIQUE(tenant_id,company_id,invitation_id),
 FOREIGN KEY(tenant_id,company_id,invitation_id) REFERENCES finance.staff_invitation(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id),
 FOREIGN KEY(tenant_id,decided_by) REFERENCES finance.identity_subject(tenant_id,id)
);
CREATE ROLE finance_onboarding_writer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_onboarding_writer;
GRANT SELECT,INSERT ON finance.staff_onboarding_request TO finance_onboarding_writer;
GRANT UPDATE(status,decided_at,decided_by,actor_id,version) ON finance.staff_onboarding_request TO finance_onboarding_writer;
ALTER TABLE finance.staff_onboarding_request ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.staff_onboarding_request FORCE ROW LEVEL SECURITY;
CREATE POLICY onboarding_scope ON finance.staff_onboarding_request TO finance_onboarding_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
GRANT SELECT ON finance.staff_invitation TO finance_onboarding_writer;
GRANT UPDATE(status,expires_at,version) ON finance.staff_invitation TO finance_onboarding_writer;
CREATE POLICY onboarding_invitation ON finance.staff_invitation TO finance_onboarding_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
GRANT SELECT ON finance.company,finance.tenant,finance.identity_subject TO finance_onboarding_writer;
CREATE POLICY onboarding_company ON finance.company FOR SELECT TO finance_onboarding_writer USING(tenant_id=finance.scope_tenant() AND id=finance.scope_company());
CREATE POLICY onboarding_tenant ON finance.tenant FOR SELECT TO finance_onboarding_writer USING(id=finance.scope_tenant());
CREATE POLICY onboarding_identity_read ON finance.identity_subject FOR SELECT TO finance_onboarding_writer USING(tenant_id=finance.scope_tenant());
GRANT INSERT(tenant_id,id,issuer,subject,display_name,email,profile_issued_at) ON finance.identity_subject TO finance_onboarding_writer;
CREATE POLICY onboarding_identity_insert ON finance.identity_subject FOR INSERT TO finance_onboarding_writer WITH CHECK(tenant_id=finance.scope_tenant() AND active);
GRANT EXECUTE ON FUNCTION finance.membership_admin_allowed(uuid,uuid,uuid),finance.membership_create(uuid,uuid,uuid,uuid,text[]) TO finance_onboarding_writer;

CREATE FUNCTION finance.staff_invitation_issue(t uuid,c uuid,a uuid,rid uuid,expected bigint,issue boolean)
RETURNS bigint LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v bigint;
BEGIN
 IF NOT coalesce(finance.membership_admin_allowed(t,c,a),false) OR issue IS NULL OR expected IS NULL THEN RAISE EXCEPTION 'forbidden' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('staff-admin/'||t::text||'/'||c::text,0));
 UPDATE finance.staff_invitation SET status=CASE WHEN issue THEN 'ISSUED' ELSE 'CANCELLED' END,
  expires_at=CASE WHEN issue THEN clock_timestamp()+interval '7 days' ELSE expires_at END,version=version+1
 WHERE tenant_id=t AND company_id=c AND id=rid AND version=expected
  AND (CASE WHEN issue THEN status='DRAFT' ELSE status IN ('DRAFT','ISSUED') END)
 RETURNING version INTO v;
 IF v IS NULL THEN RAISE EXCEPTION 'stale invitation' USING ERRCODE='23514'; END IF;
 RETURN v;
END $$;
ALTER FUNCTION finance.staff_invitation_issue(uuid,uuid,uuid,uuid,bigint,boolean) OWNER TO finance_onboarding_writer;
REVOKE ALL ON FUNCTION finance.staff_invitation_issue(uuid,uuid,uuid,uuid,bigint,boolean) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.staff_invitation_issue(uuid,uuid,uuid,uuid,bigint,boolean) TO finance_runtime;

-- Called only from the existing JWT verifier, after verified email and identity claims.
-- No candidate can receive membership or access through this endpoint.
CREATE FUNCTION finance.staff_onboarding_claim(t uuid,c uuid,iss text,sub text,mail text,nm text,issued bigint,deadline bigint)
RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE invite uuid; result uuid; prior finance.staff_onboarding_request%ROWTYPE;
BEGIN
 IF NOT coalesce(t=finance.scope_tenant() AND c=finance.scope_company()
 AND iss=NULLIF(current_setting('finance.identity_issuer',true),'')
 AND sub=NULLIF(current_setting('finance.identity_subject',true),'')
 AND length(iss) BETWEEN 1 AND 512 AND length(sub) BETWEEN 1 AND 256
 AND issued>=0 AND extract(epoch FROM clock_timestamp())<deadline
 AND EXISTS(SELECT 1 FROM finance.tenant WHERE id=t AND active)
 AND EXISTS(SELECT 1 FROM finance.company WHERE tenant_id=t AND id=c AND active),false) THEN RAISE EXCEPTION 'forbidden' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('staff-admin/'||t::text||'/'||c::text,0));
 SELECT id INTO invite FROM finance.staff_invitation WHERE tenant_id=t AND company_id=c AND lower(email)=lower(mail) AND status='ISSUED' AND expires_at>clock_timestamp();
 IF invite IS NULL THEN RETURN NULL; END IF;
 SELECT * INTO prior FROM finance.staff_onboarding_request WHERE tenant_id=t AND company_id=c AND invitation_id=invite;
 IF FOUND THEN
  IF prior.issuer=iss AND prior.subject=sub AND prior.status='PENDING' THEN RETURN prior.id; END IF;
  RETURN NULL;
 END IF;
 result:=gen_random_uuid();
 INSERT INTO finance.staff_onboarding_request(tenant_id,company_id,id,invitation_id,issuer,subject,email,display_name,profile_issued_at)
 VALUES(t,c,result,invite,iss,sub,lower(mail),nm,issued);
 RETURN result;
END $$;
ALTER FUNCTION finance.staff_onboarding_claim(uuid,uuid,text,text,text,text,bigint,bigint) OWNER TO finance_onboarding_writer;
REVOKE ALL ON FUNCTION finance.staff_onboarding_claim(uuid,uuid,text,text,text,text,bigint,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.staff_onboarding_claim(uuid,uuid,text,text,text,text,bigint,bigint) TO finance_identity_lookup;

CREATE FUNCTION finance.staff_onboarding_list(t uuid,c uuid,a uuid,after_id uuid,n integer)
RETURNS SETOF jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF NOT coalesce(finance.membership_admin_allowed(t,c,a),false) OR after_id IS NULL OR n IS NULL OR n NOT BETWEEN 1 AND 100 THEN RAISE EXCEPTION 'forbidden' USING ERRCODE='42501'; END IF;
 RETURN QUERY SELECT jsonb_build_object('id',r.id,'invitation_id',r.invitation_id,'email',r.email,'display_name',r.display_name,'status',r.status,'version',r.version,'actor_id',r.actor_id,'created_at',r.created_at)
 FROM finance.staff_onboarding_request r WHERE r.tenant_id=t AND r.company_id=c AND r.id>after_id ORDER BY r.id LIMIT n;
END $$;
ALTER FUNCTION finance.staff_onboarding_list(uuid,uuid,uuid,uuid,integer) OWNER TO finance_onboarding_writer;
REVOKE ALL ON FUNCTION finance.staff_onboarding_list(uuid,uuid,uuid,uuid,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.staff_onboarding_list(uuid,uuid,uuid,uuid,integer) TO finance_runtime;

-- Explicit admin approval grants only REQUESTER, never an admin or approver role.
CREATE FUNCTION finance.staff_onboarding_decide(t uuid,c uuid,a uuid,rid uuid,expected bigint,approve boolean)
RETURNS bigint LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE r finance.staff_onboarding_request%ROWTYPE; target uuid; v bigint; active_identity boolean;
BEGIN
 IF NOT coalesce(finance.membership_admin_allowed(t,c,a),false) OR approve IS NULL OR expected IS NULL THEN RAISE EXCEPTION 'forbidden' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('staff-admin/'||t::text||'/'||c::text,0));
 SELECT * INTO r FROM finance.staff_onboarding_request WHERE tenant_id=t AND company_id=c AND id=rid AND status='PENDING' AND version=expected FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'stale request' USING ERRCODE='23514'; END IF;
 IF approve THEN
  IF NOT EXISTS(SELECT 1 FROM finance.staff_invitation WHERE tenant_id=t AND company_id=c AND id=r.invitation_id AND status='ISSUED' AND expires_at>clock_timestamp() AND lower(email)=r.email) THEN RAISE EXCEPTION 'invalid invitation' USING ERRCODE='23514'; END IF;
  SELECT id,active INTO target,active_identity FROM finance.identity_subject WHERE tenant_id=t AND issuer=r.issuer AND subject=r.subject;
  IF target IS NULL THEN
   target:=gen_random_uuid();
   INSERT INTO finance.identity_subject(tenant_id,id,issuer,subject,display_name,email,profile_issued_at) VALUES(t,target,r.issuer,r.subject,r.display_name,r.email,r.profile_issued_at);
  ELSIF NOT active_identity OR target=a THEN RAISE EXCEPTION 'invalid identity' USING ERRCODE='23514'; END IF;
  PERFORM finance.membership_create(t,c,a,target,ARRAY['REQUESTER']::text[]);
 END IF;
 UPDATE finance.staff_invitation SET status=CASE WHEN approve THEN 'ACCEPTED' ELSE 'CANCELLED' END,version=version+1
 WHERE tenant_id=t AND company_id=c AND id=r.invitation_id AND status='ISSUED';
 UPDATE finance.staff_onboarding_request SET status=CASE WHEN approve THEN 'APPROVED' ELSE 'REJECTED' END,
 decided_by=a,decided_at=clock_timestamp(),actor_id=target,version=version+1
 WHERE tenant_id=t AND company_id=c AND id=rid RETURNING version INTO v;
 RETURN v;
END $$;
ALTER FUNCTION finance.staff_onboarding_decide(uuid,uuid,uuid,uuid,bigint,boolean) OWNER TO finance_onboarding_writer;
REVOKE ALL ON FUNCTION finance.staff_onboarding_decide(uuid,uuid,uuid,uuid,bigint,boolean) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.staff_onboarding_decide(uuid,uuid,uuid,uuid,bigint,boolean) TO finance_runtime;

-- Current effective grants, for own identity only. Read-only, no legacy authorization widening.
CREATE FUNCTION finance.staff_effective_grants(t uuid,c uuid,a uuid)
RETURNS TABLE(capability text,department_id uuid,valid_from bigint,valid_until bigint)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT DISTINCT v.cap,ra.department_id,extract(epoch FROM ra.valid_from)::bigint,extract(epoch FROM ra.valid_until)::bigint
 FROM finance.staff_role_assignment ra
 JOIN finance.staff_role r ON (r.tenant_id,r.company_id,r.id,r.version)=(ra.tenant_id,ra.company_id,ra.role_id,ra.role_version)
 JOIN finance.staff_role_revision rv ON (rv.tenant_id,rv.company_id,rv.role_id,rv.version)=(ra.tenant_id,ra.company_id,ra.role_id,ra.role_version)
 CROSS JOIN LATERAL unnest(rv.capabilities) v(cap)
 WHERE t=finance.scope_tenant() AND c=finance.scope_company() AND a=finance.scope_actor()
 AND ra.tenant_id=t AND ra.company_id=c AND ra.actor_id=a AND ra.active AND r.active
 AND ra.valid_from<=statement_timestamp() AND (ra.valid_until IS NULL OR ra.valid_until>statement_timestamp())
 AND EXISTS(SELECT 1 FROM finance.membership m JOIN finance.identity_subject i ON (i.tenant_id,i.id)=(m.tenant_id,m.actor_id) WHERE m.tenant_id=t AND m.company_id=c AND m.actor_id=a AND i.active AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
 AND EXISTS(SELECT 1 FROM finance.company WHERE tenant_id=t AND id=c AND active)
 AND (ra.department_id IS NULL OR EXISTS(SELECT 1 FROM finance.staff_department sd JOIN finance.organization_unit u ON (u.tenant_id,u.company_id,u.id)=(sd.tenant_id,sd.company_id,sd.department_id) WHERE sd.tenant_id=t AND sd.company_id=c AND sd.actor_id=a AND sd.department_id=ra.department_id AND sd.active AND u.active AND u.kind='DEPARTMENT'))
 AND NOT (ra.department_id IS NULL AND v.cap IN ('READ_DEPARTMENT','CONFIRM_DEPARTMENT'))
 AND NOT (ra.department_id IS NOT NULL AND v.cap='READ_COMPANY')
$$;
ALTER FUNCTION finance.staff_effective_grants(uuid,uuid,uuid) OWNER TO finance_membership_writer;
REVOKE ALL ON FUNCTION finance.staff_effective_grants(uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.staff_effective_grants(uuid,uuid,uuid) TO finance_runtime;
