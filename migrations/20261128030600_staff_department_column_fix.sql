-- Correct ambiguous department kind in already-applied migration 64; preserve its checksum.
CREATE OR REPLACE FUNCTION finance.staff_admin_save(t uuid,c uuid,a uuid,cmd uuid,corr uuid,req_key text,kind text,rid uuid,expected bigint,payload jsonb)
RETURNS bigint LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v bigint; prior finance.staff_admin_change%ROWTYPE; target uuid; dept uuid; pos uuid; roleid uuid; rv bigint; caps text[]; enabled boolean; req jsonb;
BEGIN
 IF NOT coalesce(finance.membership_admin_allowed(t,c,a),false) OR rid IS NULL OR rid='00000000-0000-0000-0000-000000000000' OR req_key IS NULL OR length(req_key) NOT BETWEEN 1 AND 128 OR jsonb_typeof(payload) IS DISTINCT FROM 'object' OR (expected IS NOT NULL AND expected NOT BETWEEN 0 AND 9007199254740990) THEN RAISE EXCEPTION 'invalid or forbidden' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('staff-admin/'||t::text||'/'||c::text,0));
 req:=jsonb_build_object('expected',expected,'payload',payload);
 SELECT * INTO prior FROM finance.staff_admin_change WHERE tenant_id=t AND company_id=c AND actor_id=a AND request_key=req_key;
 IF FOUND THEN
  IF prior.resource_kind<>kind OR prior.resource_id<>rid OR prior.request<>req THEN RAISE EXCEPTION 'request key conflict' USING ERRCODE='23514'; END IF;
  RETURN prior.result_version;
 END IF;
 enabled:=coalesce((payload->>'active')::boolean,true);
 IF kind='positions' THEN
  IF expected IS NULL THEN
   INSERT INTO finance.staff_position(tenant_id,company_id,id,name,active) VALUES(t,c,rid,trim(payload->>'name'),enabled) RETURNING version INTO v;
  ELSE
   UPDATE finance.staff_position SET name=trim(payload->>'name'),active=enabled,version=version+1 WHERE tenant_id=t AND company_id=c AND id=rid AND version=expected RETURNING version INTO v;
  END IF;
 ELSIF kind='invitations' THEN
  IF expected IS NULL THEN
   INSERT INTO finance.staff_invitation(tenant_id,company_id,id,email) VALUES(t,c,rid,lower(trim(payload->>'email'))) RETURNING version INTO v;
  ELSE
   UPDATE finance.staff_invitation SET status='CANCELLED',version=version+1 WHERE tenant_id=t AND company_id=c AND id=rid AND version=expected AND status='DRAFT' RETURNING version INTO v;
  END IF;
 ELSIF kind='departments' THEN
  target:=(payload->>'actor_id')::uuid; dept:=(payload->>'department_id')::uuid; pos:=(payload->>'position_id')::uuid;
  IF enabled AND (NOT EXISTS(SELECT 1 FROM finance.membership m JOIN finance.identity_subject i ON (i.tenant_id,i.id)=(m.tenant_id,m.actor_id) WHERE m.tenant_id=t AND m.company_id=c AND m.actor_id=target AND i.active AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
   OR NOT EXISTS(SELECT 1 FROM finance.organization_unit WHERE tenant_id=t AND company_id=c AND id=dept AND active AND finance.organization_unit.kind='DEPARTMENT')
   OR (pos IS NOT NULL AND NOT EXISTS(SELECT 1 FROM finance.staff_position WHERE tenant_id=t AND company_id=c AND id=pos AND active))) THEN RAISE EXCEPTION 'inactive target or invalid department' USING ERRCODE='23514'; END IF;
  IF expected IS NULL THEN
   IF NOT enabled THEN RAISE EXCEPTION 'inactive creation' USING ERRCODE='23514'; END IF;
   INSERT INTO finance.staff_department(tenant_id,company_id,id,actor_id,department_id,position_id,is_primary,active)
    VALUES(t,c,rid,target,dept,pos,coalesce((payload->>'is_primary')::boolean,false),enabled) RETURNING version INTO v;
  ELSE
   UPDATE finance.staff_department SET position_id=pos,is_primary=coalesce((payload->>'is_primary')::boolean,false),active=enabled,version=version+1
    WHERE tenant_id=t AND company_id=c AND id=rid AND version=expected AND actor_id=target AND department_id=dept RETURNING version INTO v;
  END IF;
 ELSIF kind='roles' THEN
  SELECT array_agg(DISTINCT x ORDER BY x) INTO caps FROM jsonb_array_elements_text(payload->'capabilities') x;
  IF caps IS NULL OR cardinality(caps)<>jsonb_array_length(payload->'capabilities') THEN RAISE EXCEPTION 'invalid capabilities' USING ERRCODE='23514'; END IF;
  IF expected IS NULL THEN
   INSERT INTO finance.staff_role(tenant_id,company_id,id,name,active) VALUES(t,c,rid,trim(payload->>'name'),enabled) RETURNING version INTO v;
  ELSE
   UPDATE finance.staff_role SET name=trim(payload->>'name'),active=enabled,version=version+1 WHERE tenant_id=t AND company_id=c AND id=rid AND version=expected RETURNING version INTO v;
  END IF;
  IF v IS NOT NULL THEN INSERT INTO finance.staff_role_revision(tenant_id,company_id,role_id,version,capabilities) VALUES(t,c,rid,v,caps); END IF;
 ELSIF kind='assignments' THEN
  IF expected IS NULL THEN
   target:=(payload->>'actor_id')::uuid; dept:=(payload->>'department_id')::uuid; roleid:=(payload->>'role_id')::uuid; rv:=(payload->>'role_version')::bigint;
   IF target=a OR NOT enabled OR NOT EXISTS(SELECT 1 FROM finance.membership m JOIN finance.identity_subject i ON (i.tenant_id,i.id)=(m.tenant_id,m.actor_id) WHERE m.tenant_id=t AND m.company_id=c AND m.actor_id=target AND i.active AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))
    OR NOT EXISTS(SELECT 1 FROM finance.staff_role WHERE tenant_id=t AND company_id=c AND id=roleid AND active AND version=rv)
    OR (dept IS NOT NULL AND NOT EXISTS(SELECT 1 FROM finance.staff_department sd JOIN finance.organization_unit u ON (u.tenant_id,u.company_id,u.id)=(sd.tenant_id,sd.company_id,sd.department_id) WHERE sd.tenant_id=t AND sd.company_id=c AND sd.actor_id=target AND sd.department_id=dept AND sd.active AND u.active AND u.kind='DEPARTMENT'))
   THEN RAISE EXCEPTION 'invalid role assignment' USING ERRCODE='23514'; END IF;
   SELECT capabilities INTO caps FROM finance.staff_role_revision WHERE tenant_id=t AND company_id=c AND role_id=roleid AND version=rv;
   IF (dept IS NULL AND caps && ARRAY['READ_DEPARTMENT','CONFIRM_DEPARTMENT']::text[]) OR (dept IS NOT NULL AND 'READ_COMPANY'=ANY(caps)) THEN RAISE EXCEPTION 'incompatible capability scope' USING ERRCODE='23514'; END IF;
   INSERT INTO finance.staff_role_assignment(tenant_id,company_id,id,actor_id,role_id,role_version,department_id,valid_until)
    VALUES(t,c,rid,target,roleid,rv,dept,(payload->>'valid_until')::timestamptz) RETURNING version INTO v;
  ELSE
   UPDATE finance.staff_role_assignment SET active=false,version=version+1 WHERE tenant_id=t AND company_id=c AND id=rid AND version=expected AND active RETURNING version INTO v;
  END IF;
 ELSE RAISE EXCEPTION 'unknown resource' USING ERRCODE='22023';
 END IF;
 IF v IS NULL THEN RAISE EXCEPTION 'stale version' USING ERRCODE='23514'; END IF;
 INSERT INTO finance.staff_admin_change(tenant_id,company_id,actor_id,command_id,correlation_id,request_key,resource_kind,resource_id,request,result_version)
 VALUES(t,c,a,cmd,corr,req_key,kind,rid,req,v);
 RETURN v;
END $$;
