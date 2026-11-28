-- Evaluate actual published policy rules in the existing fenced preflight
-- transaction, preserving structural blocking until all policy/finance gates
-- have authoritative inputs. No new approval route or automatic READY.
ALTER TABLE finance.check_result ADD COLUMN rule_version bigint NOT NULL DEFAULT 0 CHECK(rule_version>=0),
 ADD COLUMN evidence_ids uuid[] NOT NULL DEFAULT ARRAY[]::uuid[] CHECK(cardinality(evidence_ids)<=256);
GRANT SELECT ON finance.rule_version TO finance_workflow_writer;
CREATE POLICY workflow_published_rules_read ON finance.rule_version FOR SELECT TO finance_workflow_writer
 USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND state='PUBLISHED');

CREATE FUNCTION finance.runtime_preflight_rule_material(t uuid,c uuid,j uuid,f bigint,w uuid)
RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE p finance.preflight_request%ROWTYPE; ds finance.dossier%ROWTYPE; rules jsonb; fields jsonb;
BEGIN
 IF (t,c) IS DISTINCT FROM (finance.scope_tenant(),finance.scope_company())
 OR NOT finance.worker_scope_allowed(t,c)
 THEN RAISE EXCEPTION 'worker authority' USING ERRCODE='42501'; END IF;
 PERFORM 1 FROM finance.job WHERE (tenant_id,company_id,id,fence,leased_by)=(t,c,j,f,w)
 AND state='LEASED' AND lease_until>clock_timestamp() FOR UPDATE;
 IF NOT FOUND THEN RAISE EXCEPTION 'stale preflight lease' USING ERRCODE='40001'; END IF;
 SELECT * INTO p FROM finance.preflight_request WHERE (tenant_id,company_id,job_id)=(t,c,j);
 IF NOT FOUND THEN RAISE EXCEPTION 'unknown preflight request' USING ERRCODE='P0002'; END IF;
 PERFORM set_config('finance.actor_id',p.actor_id::text,true);
 SELECT * INTO ds FROM finance.dossier WHERE (tenant_id,company_id,id)=(t,c,p.dossier_id) FOR UPDATE;
 IF NOT FOUND OR ds.owner_id<>p.actor_id OR ds.version<>p.dossier_version OR ds.status<>'DRAFT'
 OR NOT EXISTS(SELECT 1 FROM finance.dossier_revision dr WHERE (dr.tenant_id,dr.company_id,dr.dossier_id,dr.id,dr.number)=(t,c,ds.id,p.revision_id,ds.revision))
 OR p.business_day<>(clock_timestamp() AT TIME ZONE 'UTC')::date
 OR NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,p.actor_id)
 AND m.role='REQUESTER' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp()))
 THEN RAISE EXCEPTION 'stale preflight material' USING ERRCODE='23514'; END IF;
 PERFORM finance.prototype_policy_lock(t,c,ds.kind);
 IF p.policy_id IS DISTINCT FROM finance.prototype_effective_policy(t,c,ds.kind,p.business_day)
 THEN RAISE EXCEPTION 'stale policy' USING ERRCODE='23514'; END IF;
 SELECT coalesce(jsonb_agg(jsonb_build_object('id',r.id,'name',r.rule_key,'version',r.version,'severity',r.severity,'definition',r.definition) ORDER BY r.rule_key,r.id),'[]'::jsonb) INTO rules
 FROM (SELECT * FROM finance.rule_version WHERE (tenant_id,company_id,policy_id)=(t,c,p.policy_id) AND state='PUBLISHED' ORDER BY rule_key,id LIMIT 1001) r;
 IF jsonb_array_length(rules)>1000 OR octet_length(rules::text)>4194304 THEN RAISE EXCEPTION 'rule material limit' USING ERRCODE='23514'; END IF;
 SELECT coalesce(jsonb_agg(row.data ORDER BY row.document,row.path),'[]'::jsonb) INTO fields
 FROM (
 SELECT o.document_id AS document,f.field_path AS path,
 jsonb_build_object('document',o.document_id,'path',f.field_path,
 'value',CASE WHEN q.disposition IN ('CONFIRMED_CANDIDATE','CORRECTED') THEN q.normalized_value END,
 'verified',coalesce(q.disposition IN ('CONFIRMED_CANDIDATE','CORRECTED') AND cardinality(q.evidence_ids)>0,false),
 'evidence',coalesce(to_jsonb(q.evidence_ids),'[]'::jsonb)) AS data
 FROM finance.document_ref dr JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(dr.tenant_id,dr.company_id,dr.id)
 JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id)=(dr.tenant_id,dr.company_id,dr.id)
 JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.document_id,o.revision_id,o.generation)=(dr.tenant_id,dr.company_id,dr.id,dr.revision_id,b.generation)
 JOIN finance.extraction_run x ON (x.tenant_id,x.company_id,x.ocr_run_id)=(o.tenant_id,o.company_id,o.id)
 JOIN finance.extracted_field f ON (f.tenant_id,f.company_id,f.run_id)=(x.tenant_id,x.company_id,x.id)
 LEFT JOIN LATERAL (
 SELECT q.disposition,q.normalized_value,q.evidence_ids FROM finance.field_decision q
 WHERE (q.tenant_id,q.company_id,q.run_id,q.field_path,q.dossier_id,q.revision_id)=(f.tenant_id,f.company_id,f.run_id,f.field_path,p.dossier_id,p.revision_id)
 AND q.dossier_version<=p.dossier_version ORDER BY q.dossier_version DESC,q.created_at DESC,q.id DESC LIMIT 1
 ) q ON true
 WHERE (dr.tenant_id,dr.company_id,dr.dossier_id,dr.revision_id)=(t,c,p.dossier_id,p.revision_id)
 AND b.scan_state='CLEAN' AND u.state='FINALIZED' AND u.generation=b.generation AND o.source_digest=b.sha256_digest
 AND NOT EXISTS(SELECT 1 FROM finance.ocr_run newer WHERE (newer.tenant_id,newer.company_id,newer.document_id,newer.revision_id)=(o.tenant_id,o.company_id,o.document_id,o.revision_id)
 AND (newer.created_at,newer.id)>(o.created_at,o.id))
 ORDER BY o.document_id,f.field_path LIMIT 5001
 ) row;
 IF jsonb_array_length(fields)>5000 OR octet_length(fields::text)>8388608 THEN RAISE EXCEPTION 'field material limit' USING ERRCODE='23514'; END IF;
 RETURN jsonb_build_object('policy',p.policy_id,'revision',p.revision_id,'version',p.dossier_version,'rules',rules,'fields',fields);
END $$;
ALTER FUNCTION finance.runtime_preflight_rule_material(uuid,uuid,uuid,bigint,uuid) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.runtime_preflight_rule_material(uuid,uuid,uuid,bigint,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_preflight_rule_material(uuid,uuid,uuid,bigint,uuid) TO finance_job_worker;

CREATE FUNCTION finance.complete_preflight_with_rules(t uuid,c uuid,j uuid,f bigint,w uuid,material jsonb,results jsonb)
RETURNS void LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE current_material jsonb; p finance.preflight_request%ROWTYPE; result jsonb; rule jsonb; evidence uuid[]; seen uuid[]:=ARRAY[]::uuid[];
BEGIN
 current_material:=finance.runtime_preflight_rule_material(t,c,j,f,w);
 IF current_material IS DISTINCT FROM material OR jsonb_typeof(results) IS DISTINCT FROM 'array'
 OR jsonb_array_length(results)<>jsonb_array_length(material->'rules') OR octet_length(results::text)>2097152
 THEN RAISE EXCEPTION 'stale rule evaluation' USING ERRCODE='23514'; END IF;
 SELECT * INTO STRICT p FROM finance.preflight_request WHERE (tenant_id,company_id,job_id)=(t,c,j);
 -- Existing completion writes structural checks and settles the same job;
 -- all writes, including the rule results below, remain in this transaction.
 PERFORM finance.complete_preflight_job(t,c,j,f,w);
 FOR result IN SELECT value FROM jsonb_array_elements(results) LOOP
  SELECT value INTO rule FROM jsonb_array_elements(material->'rules') WHERE value->>'id'=result->>'id';
  IF NOT FOUND OR (result->>'version')::bigint IS DISTINCT FROM (rule->>'version')::bigint
  OR (result->>'id')::uuid=ANY(seen) OR result->>'outcome' IS NULL OR result->>'outcome' NOT IN ('PASS','FAIL','UNKNOWN','ERROR','NOT_APPLICABLE')
  OR result->>'detail' IS NULL OR result->>'detail' NOT IN ('RULE_EVALUATED','RULE_COMPILE_ERROR')
  OR jsonb_typeof(result->'evidence') IS DISTINCT FROM 'array' OR jsonb_array_length(result->'evidence')>256
  THEN RAISE EXCEPTION 'invalid rule result' USING ERRCODE='23514'; END IF;
  seen:=array_append(seen,(result->>'id')::uuid);
  SELECT coalesce(array_agg(DISTINCT x::uuid ORDER BY x::uuid),ARRAY[]::uuid[]) INTO evidence FROM jsonb_array_elements_text(result->'evidence') x;
  IF cardinality(evidence)<>jsonb_array_length(result->'evidence') OR EXISTS(
   SELECT 1 FROM unnest(evidence) e WHERE NOT EXISTS(
    SELECT 1 FROM jsonb_array_elements(material->'fields') fld
    CROSS JOIN LATERAL jsonb_array_elements_text(fld->'evidence') ev
    WHERE fld->>'verified'='true' AND ev=e::text))
  THEN RAISE EXCEPTION 'unbound rule evidence' USING ERRCODE='23514'; END IF;
  INSERT INTO finance.check_result(tenant_id,company_id,run_id,rule_key,outcome,severity,detail_code,rule_version,evidence_ids)
  VALUES(t,c,p.run_id,'dsl.'||(rule->>'name'),result->>'outcome',rule->>'severity',result->>'detail',(rule->>'version')::bigint,evidence);
 END LOOP;
END $$;
ALTER FUNCTION finance.complete_preflight_with_rules(uuid,uuid,uuid,bigint,uuid,jsonb,jsonb) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.complete_preflight_with_rules(uuid,uuid,uuid,bigint,uuid,jsonb,jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.complete_preflight_with_rules(uuid,uuid,uuid,bigint,uuid,jsonb,jsonb) TO finance_job_worker;
