-- E02/E03 membership aggregate command boundary.
ALTER TABLE finance.membership
 ADD COLUMN version bigint NOT NULL DEFAULT 0
 CHECK(version BETWEEN 0 AND 9007199254740991);

CREATE ROLE finance_membership_writer
 NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
GRANT USAGE ON SCHEMA finance TO finance_membership_writer;
GRANT SELECT,INSERT ON finance.membership TO finance_membership_writer;
GRANT UPDATE(active,valid_from,valid_until,version) ON finance.membership TO finance_membership_writer;
GRANT SELECT ON finance.identity_subject,finance.company TO finance_membership_writer;

CREATE POLICY membership_writer_read ON finance.membership
 FOR SELECT TO finance_membership_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY membership_writer_insert ON finance.membership
 FOR INSERT TO finance_membership_writer
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY membership_writer_update ON finance.membership
 FOR UPDATE TO finance_membership_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company())
 WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY membership_writer_subject_read ON finance.identity_subject
 FOR SELECT TO finance_membership_writer
 USING(tenant_id=finance.scope_tenant());
CREATE POLICY membership_writer_company_read ON finance.company
 FOR SELECT TO finance_membership_writer
 USING(tenant_id=finance.scope_tenant() AND id=finance.scope_company());

CREATE FUNCTION finance.membership_admin_allowed(t uuid,c uuid,a uuid)
RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT t=finance.scope_tenant() AND c=finance.scope_company() AND a=finance.scope_actor()
 AND EXISTS(
  SELECT 1 FROM finance.membership m
  WHERE m.tenant_id=t AND m.company_id=c AND m.actor_id=a
   AND m.role='ORG_ADMIN' AND m.active
   AND m.valid_from<=statement_timestamp()
   AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())
 )
$$;
ALTER FUNCTION finance.membership_admin_allowed(uuid,uuid,uuid)
 OWNER TO finance_membership_writer;
REVOKE ALL ON FUNCTION finance.membership_admin_allowed(uuid,uuid,uuid) FROM PUBLIC;

CREATE FUNCTION finance.runtime_memberships(
 t uuid,c uuid,a uuid,after_id uuid,only_id uuid,n integer
) RETURNS TABLE(
 id uuid,subject_id uuid,company_id uuid,roles text[],active boolean,version bigint
)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
 IF after_id IS NULL OR n IS NULL OR n NOT BETWEEN 1 AND 100
 OR NOT finance.membership_admin_allowed(t,c,a)
 THEN RAISE EXCEPTION 'membership read forbidden or invalid' USING ERRCODE='42501'; END IF;
 RETURN QUERY
 SELECT m.actor_id,m.actor_id,m.company_id,array_agg(m.role ORDER BY m.role),
  bool_or(m.active AND m.valid_from<=statement_timestamp()
   AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp())),
  max(m.version)
 FROM finance.membership m
 WHERE m.tenant_id=t AND m.company_id=c AND m.actor_id>after_id
  AND (only_id IS NULL OR m.actor_id=only_id)
 GROUP BY m.actor_id,m.company_id
 ORDER BY m.actor_id
 LIMIT n;
END $$;
ALTER FUNCTION finance.runtime_memberships(uuid,uuid,uuid,uuid,uuid,integer)
 OWNER TO finance_membership_writer;
REVOKE ALL ON FUNCTION finance.runtime_memberships(uuid,uuid,uuid,uuid,uuid,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_memberships(uuid,uuid,uuid,uuid,uuid,integer)
 TO finance_runtime;

CREATE FUNCTION finance.membership_create(
 t uuid,c uuid,a uuid,target uuid,roles text[]
) RETURNS bigint
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE role_name text;
BEGIN
 IF target IS NULL OR target=a OR roles IS NULL OR cardinality(roles) NOT BETWEEN 1 AND 6
 OR NOT finance.membership_admin_allowed(t,c,a)
 THEN RAISE EXCEPTION 'membership create forbidden or invalid' USING ERRCODE='42501'; END IF;
 IF (SELECT count(DISTINCT x) FROM unnest(roles) x)<>cardinality(roles)
 OR EXISTS(SELECT 1 FROM unnest(roles) x
  WHERE x NOT IN ('ORG_ADMIN','REQUESTER','ACCOUNTANT','AUDITOR','FINANCE_APPROVER','POLICY_ADMIN'))
 THEN RAISE EXCEPTION 'invalid membership roles' USING ERRCODE='23514'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(
  'finance.membership/v1/'||t::text||'/'||c::text||'/'||target::text,0));
 IF NOT EXISTS(SELECT 1 FROM finance.company WHERE tenant_id=t AND id=c AND active)
 OR NOT EXISTS(SELECT 1 FROM finance.identity_subject WHERE tenant_id=t AND id=target AND active)
 THEN RAISE EXCEPTION 'inactive membership target' USING ERRCODE='23514'; END IF;
 IF EXISTS(SELECT 1 FROM finance.membership
  WHERE tenant_id=t AND company_id=c AND actor_id=target)
 THEN RAISE EXCEPTION 'membership already exists' USING ERRCODE='23514'; END IF;
 FOREACH role_name IN ARRAY roles LOOP
  INSERT INTO finance.membership(
   tenant_id,company_id,actor_id,role,active,valid_from,valid_until,version
  ) VALUES(t,c,target,role_name,true,clock_timestamp(),NULL,0);
 END LOOP;
 RETURN 0;
END $$;
ALTER FUNCTION finance.membership_create(uuid,uuid,uuid,uuid,text[])
 OWNER TO finance_membership_writer;
REVOKE ALL ON FUNCTION finance.membership_create(uuid,uuid,uuid,uuid,text[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.membership_create(uuid,uuid,uuid,uuid,text[]) TO finance_runtime;

CREATE FUNCTION finance.membership_revoke(
 t uuid,c uuid,a uuid,target uuid,expected bigint
) RETURNS bigint
LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE current_version bigint; next_version bigint;
BEGIN
 IF target IS NULL OR target=a OR expected IS NULL OR expected<0
 OR NOT finance.membership_admin_allowed(t,c,a)
 THEN RAISE EXCEPTION 'membership revoke forbidden or invalid' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended(
  'finance.membership/v1/'||t::text||'/'||c::text||'/'||target::text,0));
 PERFORM 1 FROM finance.membership
 WHERE tenant_id=t AND company_id=c AND actor_id=target
 ORDER BY role FOR UPDATE;
 SELECT max(version) INTO current_version FROM finance.membership
 WHERE tenant_id=t AND company_id=c AND actor_id=target AND active;
 IF current_version IS NULL OR current_version<>expected
 THEN RAISE EXCEPTION 'stale or inactive membership' USING ERRCODE='23514'; END IF;
 IF EXISTS(SELECT 1 FROM finance.membership
   WHERE tenant_id=t AND company_id=c AND actor_id=target AND role='ORG_ADMIN' AND active)
 AND NOT EXISTS(SELECT 1 FROM finance.membership
   WHERE tenant_id=t AND company_id=c AND actor_id<>target AND role='ORG_ADMIN' AND active
    AND valid_from<=clock_timestamp()
    AND (valid_until IS NULL OR valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'last organization administrator' USING ERRCODE='23514'; END IF;
 next_version:=expected+1;
 UPDATE finance.membership
 SET active=false,valid_until=clock_timestamp(),version=next_version
 WHERE tenant_id=t AND company_id=c AND actor_id=target AND active;
 RETURN next_version;
END $$;
ALTER FUNCTION finance.membership_revoke(uuid,uuid,uuid,uuid,bigint)
 OWNER TO finance_membership_writer;
REVOKE ALL ON FUNCTION finance.membership_revoke(uuid,uuid,uuid,uuid,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.membership_revoke(uuid,uuid,uuid,uuid,bigint) TO finance_runtime;

ALTER TABLE finance.idempotency_record DROP CONSTRAINT idempotency_record_operation_id_check,
 ADD CONSTRAINT idempotency_record_operation_id_check CHECK(operation_id IN (
 'prototype.createOrganizationUnit','prototype.updateOrganizationUnit','prototype.createDossier',
 'prototype.createDocument','form.create','form.revise','template.create','template.revise','dossier.ready',
 'rule.create','waiver.request','waiver.decide','handoff.manual','reconciliation.resolve','export.request',
 'field.decide','match.confirm','review.begin','review.decide','membership.create','membership.revoke'));
ALTER TABLE finance.idempotency_record
 ADD COLUMN membership_result_id uuid GENERATED ALWAYS AS
  (CASE WHEN operation_id IN ('membership.create','membership.revoke') THEN result_id ELSE NULL END) STORED,
 ADD FOREIGN KEY(tenant_id,membership_result_id)
  REFERENCES finance.identity_subject(tenant_id,id);

ALTER TABLE finance.audit_event DROP CONSTRAINT audit_event_action_check,
 ADD CONSTRAINT audit_event_action_check CHECK(action IN (
 'ORG_UNIT_CREATED','AUDIT_READ','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','EXPORT_COMPLETED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED'));
ALTER TABLE finance.outbox DROP CONSTRAINT outbox_event_type_check,
 ADD CONSTRAINT outbox_event_type_check CHECK(event_type IN (
 'ORG_UNIT_CREATED','ORG_UNIT_UPDATED','DOSSIER_CREATED','DOCUMENT_INTENT_CREATED','SYNTHETIC_OCR_REQUESTED',
 'SYNTHETIC_FIELD_REVIEWED','SYNTHETIC_PREFLIGHT_CHECKED','FORM_DRAFT_CREATED','FORM_DRAFT_REVISED',
 'TEMPLATE_DRAFT_CREATED','TEMPLATE_DRAFT_REVISED','DOSSIER_READY','RULE_DRAFT_CREATED',
 'WAIVER_REQUESTED','WAIVER_DECIDED','MANUAL_HANDOFF_ACKNOWLEDGED','RECONCILIATION_RESOLVED',
 'EXPORT_REQUESTED','FIELD_REVIEWED','MATCH_CONFIRMED','REVIEW_BEGUN','REVIEW_DECIDED',
 'MEMBERSHIP_CREATED','MEMBERSHIP_REVOKED'));
