-- E10-E15: bind each cap to a confirmed document identity. The rule input
-- includes current independently verified cap facts, not caller supplied totals.
CREATE OR REPLACE FUNCTION finance.bound_finance_material(t uuid,c uuid,d uuid,rev uuid,pid uuid,v bigint,ids uuid[])
RETURNS jsonb LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE gate jsonb; fields jsonb; amount_row jsonb; currency_row jsonb; n numeric; k uuid; fr finance.financial_resource%ROWTYPE; refs jsonb:='[]'; kinds text[]; expected text[]; cur text; key_row jsonb; key_path text; key_count bigint;
BEGIN
 IF (t,c) IS DISTINCT FROM (finance.scope_tenant(),finance.scope_company()) OR cardinality(ids) NOT BETWEEN 1 AND 4 OR array_position(ids,NULL::uuid) IS NOT NULL THEN RETURN NULL; END IF;
 SELECT pv.finance_gate INTO gate FROM finance.policy_version pv JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(t,c,d)
 WHERE (pv.tenant_id,pv.company_id,pv.id)=(t,c,pid) AND pv.state='PUBLISHED' AND pv.criticality_complete AND pv.dossier_kind=ds.kind
 AND pv.effective_from<=(clock_timestamp() AT TIME ZONE 'UTC')::date AND pv.effective_until>(clock_timestamp() AT TIME ZONE 'UTC')::date;
 IF gate IS NULL OR jsonb_typeof(gate->'resource_keys') IS DISTINCT FROM 'object' THEN RETURN NULL; END IF;
 SELECT jsonb_agg(row.item ORDER BY row.doc,row.path) INTO fields FROM (
 SELECT o.document_id doc,f.field_path path,jsonb_build_object('document',o.document_id,'path',f.field_path,'decision',q.id,'value',q.normalized_value,'evidence',q.evidence_ids,'verified',coalesce(q.disposition IN ('CONFIRMED_CANDIDATE','CORRECTED') AND cardinality(q.evidence_ids)>0,false),'source',encode(b.sha256_digest,'hex')) item
 FROM finance.document_ref dr JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(dr.tenant_id,dr.company_id,dr.id)
 JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id)=(dr.tenant_id,dr.company_id,dr.id)
 JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.document_id,o.revision_id,o.generation)=(dr.tenant_id,dr.company_id,dr.id,dr.revision_id,b.generation)
 JOIN finance.extraction_run x ON (x.tenant_id,x.company_id,x.ocr_run_id)=(o.tenant_id,o.company_id,o.id)
 JOIN finance.extracted_field f ON (f.tenant_id,f.company_id,f.run_id)=(x.tenant_id,x.company_id,x.id)
 LEFT JOIN LATERAL (SELECT q.id,q.disposition,q.normalized_value,q.evidence_ids FROM finance.field_decision q
 WHERE (q.tenant_id,q.company_id,q.run_id,q.field_path,q.dossier_id,q.revision_id)=(f.tenant_id,f.company_id,f.run_id,f.field_path,d,rev)
 AND q.dossier_version<=v ORDER BY q.dossier_version DESC,q.created_at DESC,q.id DESC LIMIT 1) q ON true
 WHERE (dr.tenant_id,dr.company_id,dr.dossier_id,dr.revision_id)=(t,c,d,rev)
 AND b.scan_state='CLEAN' AND u.state='FINALIZED' AND u.generation=b.generation AND b.sha256_digest=u.declared_sha256 AND o.source_digest=b.sha256_digest
 AND (f.field_path IN (gate->>'amount_path',gate->>'currency_path') OR f.field_path IN (SELECT value FROM jsonb_each_text(gate->'resource_keys')))
 AND NOT EXISTS(SELECT 1 FROM finance.ocr_run newer WHERE (newer.tenant_id,newer.company_id,newer.document_id,newer.revision_id)=(o.tenant_id,o.company_id,o.document_id,o.revision_id) AND (newer.created_at,newer.id)>(o.created_at,o.id))
 LIMIT 13) row;
 IF jsonb_array_length(fields) IS DISTINCT FROM 2+(SELECT count(DISTINCT value) FROM jsonb_each_text(gate->'resource_keys')) THEN RETURN NULL; END IF;
 SELECT value INTO amount_row FROM jsonb_array_elements(fields) WHERE value->>'path'=gate->>'amount_path';
 SELECT value INTO currency_row FROM jsonb_array_elements(fields) WHERE value->>'path'=gate->>'currency_path';
 IF amount_row IS NULL OR currency_row IS NULL OR amount_row->>'verified' IS DISTINCT FROM 'true' OR currency_row->>'verified' IS DISTINCT FROM 'true'
 OR amount_row->>'document' IS DISTINCT FROM currency_row->>'document' OR amount_row->>'value' IS NULL
 OR amount_row->>'value' !~ '^[0-9]{1,20}([.][0-9]{1,8})?$' THEN RETURN NULL; END IF;
 n:=(amount_row->>'value')::numeric; cur:=currency_row->>'value';
 IF n<=0 OR cur IS NULL OR cur !~ '^[A-Z]{3}$' THEN RETURN NULL; END IF;
 SELECT array_agg(k ORDER BY k) INTO expected FROM jsonb_array_elements_text(gate->'resource_kinds') k;
 FOR k IN SELECT DISTINCT z FROM unnest(ids) z ORDER BY z LOOP
  SELECT * INTO fr FROM finance.financial_resource r WHERE (r.tenant_id,r.company_id,r.id)=(t,c,k) FOR UPDATE;
  IF NOT FOUND OR fr.currency<>cur OR NOT fr.coverage_complete OR fr.fresh_until<=clock_timestamp()
  OR NOT EXISTS(SELECT 1 FROM finance.reference_snapshot s JOIN finance.source_coverage sc ON (sc.tenant_id,sc.company_id,sc.snapshot_id)=(s.tenant_id,s.company_id,s.id)
   WHERE (s.tenant_id,s.company_id,s.id)=(t,c,fr.snapshot_id) AND s.state='APPROVED' AND s.source_document_id IS NOT NULL AND s.fresh_until>clock_timestamp() AND sc.resource_kind=fr.resource_kind AND sc.complete)
  THEN RETURN NULL; END IF;
  key_path:=gate->'resource_keys'->>fr.resource_kind;
  SELECT count(*) INTO key_count FROM jsonb_array_elements(fields) fld WHERE fld->>'path'=key_path;
  IF key_count<>1 THEN RETURN NULL; END IF;
  SELECT value INTO key_row FROM jsonb_array_elements(fields) WHERE value->>'path'=key_path;
  IF key_row->>'verified' IS DISTINCT FROM 'true' OR key_row->>'value' IS NULL
  OR fr.source_key IS DISTINCT FROM jsonb_build_array('MANUAL_API',key_row->>'value')::text THEN RETURN NULL; END IF;
  refs:=refs||jsonb_build_array(jsonb_build_object('id',fr.id,'snapshot',fr.snapshot_id,'kind',fr.resource_kind,'key',fr.source_key));
 END LOOP;
 SELECT array_agg(z->>'kind' ORDER BY z->>'kind') INTO kinds FROM jsonb_array_elements(refs) z;
 IF kinds IS DISTINCT FROM expected OR jsonb_array_length(refs)<>cardinality(ids) THEN RETURN NULL; END IF;
 RETURN jsonb_build_object('policy',pid,'revision',rev,'amount',n,'currency',cur,'fields',fields,'resources',refs);
END $$;
ALTER FUNCTION finance.bound_finance_material(uuid,uuid,uuid,uuid,uuid,bigint,uuid[]) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.bound_finance_material(uuid,uuid,uuid,uuid,uuid,bigint,uuid[]) FROM PUBLIC;


CREATE FUNCTION finance.runtime_verified_financial_facts(t uuid,c uuid,d uuid,rev uuid,pid uuid,v bigint)
RETURNS jsonb LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE hold_id uuid; hold finance.reservation%ROWTYPE; fr finance.financial_resource%ROWTYPE;
 snap finance.reference_snapshot%ROWTYPE; row_data jsonb; held numeric; result jsonb:='[]'; prefix text; k uuid; ev jsonb; minimum numeric; all_ev jsonb:='[]'; amount_ev jsonb; mat jsonb;
BEGIN
 hold_id:=finance.current_bound_reservation(t,c,d,rev,pid,v);
 IF hold_id IS NULL THEN RETURN '[]'::jsonb; END IF;
 SELECT * INTO STRICT hold FROM finance.reservation x WHERE (x.tenant_id,x.company_id,x.id)=(t,c,hold_id);
 mat:=finance.bound_finance_material(t,c,d,rev,pid,v,hold.resource_ids);
 IF mat IS NULL THEN RETURN '[]'::jsonb; END IF;
 SELECT coalesce(jsonb_agg(DISTINCT e),'[]'::jsonb) INTO amount_ev
 FROM jsonb_array_elements(mat->'fields') fld CROSS JOIN LATERAL jsonb_array_elements(fld->'evidence') e;
 FOR k IN SELECT z FROM unnest(hold.resource_ids) z ORDER BY z LOOP
  SELECT * INTO STRICT fr FROM finance.financial_resource x WHERE (x.tenant_id,x.company_id,x.id)=(t,c,k);
  SELECT * INTO STRICT snap FROM finance.reference_snapshot x WHERE (x.tenant_id,x.company_id,x.id)=(t,c,fr.snapshot_id);
  SELECT item INTO row_data FROM jsonb_array_elements(snap.import_payload) item
   WHERE jsonb_build_array('MANUAL_API',item->>'source_key')::text=fr.source_key;
  IF row_data IS NULL OR jsonb_typeof(row_data->'evidence_ids') IS DISTINCT FROM 'array'
    OR jsonb_array_length(row_data->'evidence_ids')=0 THEN RETURN '[]'::jsonb; END IF;
  SELECT coalesce(sum(r.remaining),0) INTO held FROM finance.reservation_line l
   JOIN finance.reservation r ON (r.tenant_id,r.company_id,r.id)=(l.tenant_id,l.company_id,l.reservation_id)
   WHERE (l.tenant_id,l.company_id,l.resource_id)=(t,c,k) AND r.id<>hold_id AND r.state IN ('RESERVED','COMMITTED','PARTIAL');
  SELECT coalesce(jsonb_agg(DISTINCT e),'[]'::jsonb) INTO ev
   FROM jsonb_array_elements((row_data->'evidence_ids')||amount_ev) e;
  IF jsonb_array_length(ev)>64 THEN RETURN '[]'::jsonb; END IF;
  prefix:=lower(fr.resource_kind);
  minimum:=CASE WHEN minimum IS NULL THEN fr.cap-fr.consumed-held ELSE least(minimum,fr.cap-fr.consumed-held) END;
  all_ev:=all_ev||ev;
  result:=result||jsonb_build_array(
   jsonb_build_object('path',prefix||'.remaining','value',(fr.cap-fr.consumed-held)::text,'currency',fr.currency,'verified',true,'evidence',ev),
   jsonb_build_object('path',prefix||'.cap','value',fr.cap::text,'currency',fr.currency,'verified',true,'evidence',ev),
   jsonb_build_object('path',prefix||'.consumed','value',fr.consumed::text,'currency',fr.currency,'verified',true,'evidence',ev),
   jsonb_build_object('path',prefix||'.other_holds','value',held::text,'currency',fr.currency,'verified',true,'evidence',ev));
 END LOOP;
 SELECT coalesce(jsonb_agg(DISTINCT e),'[]'::jsonb) INTO all_ev FROM jsonb_array_elements(all_ev) e;
 IF minimum IS NULL OR jsonb_array_length(all_ev)>64 THEN RETURN '[]'::jsonb; END IF;
 RETURN result||jsonb_build_array(
  jsonb_build_object('path','finance.payable_cap','value',minimum::text,'currency',hold.currency,'verified',true,'evidence',all_ev),
  jsonb_build_object('path','finance.requested','value',hold.amount::text,'currency',hold.currency,'verified',true,'evidence',amount_ev),
  jsonb_build_object('path','reference.fresh','value','true','verified',true,'evidence',all_ev));
END $$;
ALTER FUNCTION finance.runtime_verified_financial_facts(uuid,uuid,uuid,uuid,uuid,bigint) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.runtime_verified_financial_facts(uuid,uuid,uuid,uuid,uuid,bigint) FROM PUBLIC;
CREATE OR REPLACE FUNCTION finance.runtime_preflight_rule_material(t uuid,c uuid,j uuid,f bigint,w uuid)
RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE p finance.preflight_request%ROWTYPE; ds finance.dossier%ROWTYPE; rules jsonb; fields jsonb; financial jsonb;
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
 financial:=finance.runtime_verified_financial_facts(t,c,p.dossier_id,p.revision_id,p.policy_id,p.dossier_version);
 RETURN jsonb_build_object('invoice_document_count',(SELECT count(*) FROM finance.upload_intent u WHERE (u.tenant_id,u.company_id,u.dossier_id,u.revision_id)=(t,c,p.dossier_id,p.revision_id) AND u.document_kind='INVOICE' AND u.state<>'CANCELLED'),'financial',financial,'policy',p.policy_id,'revision',p.revision_id,'version',p.dossier_version,'rules',rules,'fields',fields);
END $$;
ALTER FUNCTION finance.runtime_preflight_rule_material(uuid,uuid,uuid,bigint,uuid) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.runtime_preflight_rule_material(uuid,uuid,uuid,bigint,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_preflight_rule_material(uuid,uuid,uuid,bigint,uuid) TO finance_job_worker;


CREATE OR REPLACE FUNCTION finance.complete_preflight_with_rules(t uuid,c uuid,j uuid,f bigint,w uuid,material jsonb,results jsonb)
RETURNS void LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE current_material jsonb; p finance.preflight_request%ROWTYPE; result jsonb; rule jsonb; evidence uuid[]; seen uuid[]:=ARRAY[]::uuid[]; structural jsonb:='[]'::jsonb; item record; outcome text; detail text; blocked boolean; revision_version bigint; affected bigint; digest bytea;
BEGIN
 current_material:=finance.runtime_preflight_rule_material(t,c,j,f,w);
 IF current_material IS DISTINCT FROM material OR jsonb_typeof(results) IS DISTINCT FROM 'array'
 OR jsonb_array_length(results)<>jsonb_array_length(material->'rules') OR octet_length(results::text)>2097152
 THEN RAISE EXCEPTION 'stale rule evaluation' USING ERRCODE='23514'; END IF;
 SELECT * INTO STRICT p FROM finance.preflight_request WHERE (tenant_id,company_id,job_id)=(t,c,j);
 -- Evaluate current structural evidence, not fixed prototype outcomes.
 -- A clean blob must correspond to the exact finalized generation and digest.
 structural:=structural||jsonb_build_array(jsonb_build_object('key','policy.effective','outcome','PASS','severity','BLOCK','detail','PUBLISHED_POLICY_FOUND'));
 FOR item IN SELECT document_kind,mandatory FROM finance.checklist_item
 WHERE (tenant_id,company_id,policy_id)=(t,c,p.policy_id) ORDER BY document_kind LOOP
  IF EXISTS(SELECT 1 FROM finance.upload_intent u JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(u.tenant_id,u.company_id,u.id)
   WHERE (u.tenant_id,u.company_id,u.dossier_id,u.revision_id,u.actor_id,u.document_kind)=(t,c,p.dossier_id,p.revision_id,p.actor_id,item.document_kind)
   AND u.state='FINALIZED' AND b.generation=u.generation AND b.sha256_digest=u.declared_sha256 AND b.scan_state='CLEAN')
   THEN outcome:='PASS'; detail:='CLEAN_DOCUMENT_FOUND';
  ELSIF EXISTS(SELECT 1 FROM finance.upload_intent u WHERE (u.tenant_id,u.company_id,u.dossier_id,u.revision_id,u.document_kind)=(t,c,p.dossier_id,p.revision_id,item.document_kind) AND u.state<>'CANCELLED')
   THEN outcome:='UNKNOWN'; detail:='DOCUMENT_NOT_RELEASED';
  ELSE outcome:='FAIL'; detail:='MISSING_DOCUMENT'; END IF;
  structural:=structural||jsonb_build_array(jsonb_build_object('key','checklist.'||item.document_kind,'outcome',outcome,'severity',CASE WHEN item.mandatory THEN 'BLOCK' ELSE 'WARNING' END,'detail',detail));
 END LOOP;
 IF EXISTS(SELECT 1 FROM finance.upload_intent u LEFT JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(u.tenant_id,u.company_id,u.id)
 WHERE (u.tenant_id,u.company_id,u.dossier_id,u.revision_id)=(t,c,p.dossier_id,p.revision_id) AND u.state<>'CANCELLED'
 AND (u.state<>'FINALIZED' OR b.id IS NULL OR b.generation<>u.generation OR b.sha256_digest<>u.declared_sha256 OR b.scan_state<>'CLEAN'))
 THEN outcome:='UNKNOWN'; detail:='DOCUMENT_NOT_RELEASED'; ELSE outcome:='PASS'; detail:='CLEAN_GENERATIONS_VERIFIED'; END IF;
 structural:=structural||jsonb_build_array(jsonb_build_object('key','scan.release','outcome',outcome,'severity','BLOCK','detail',detail));
 IF jsonb_array_length(material->'rules')=0 OR EXISTS(SELECT 1 FROM finance.policy_clause pc
 WHERE (pc.tenant_id,pc.company_id,pc.policy_id)=(t,c,p.policy_id)
 AND NOT EXISTS(SELECT 1 FROM finance.checklist_item ci WHERE (ci.tenant_id,ci.company_id,ci.policy_id,ci.clause_id)=(t,c,p.policy_id,pc.id))
 AND NOT EXISTS(SELECT 1 FROM finance.rule_version rv WHERE (rv.tenant_id,rv.company_id,rv.policy_id,rv.clause_id)=(t,c,p.policy_id,pc.id) AND rv.state='PUBLISHED')
 AND NOT EXISTS(SELECT 1 FROM finance.policy_critical_field cf WHERE (cf.tenant_id,cf.company_id,cf.policy_id,cf.clause_id)=(t,c,p.policy_id,pc.id)))
 THEN outcome:='UNKNOWN'; detail:='POLICY_RULE_COVERAGE_INCOMPLETE'; ELSE outcome:='PASS'; detail:='PUBLISHED_RULE_COVERAGE_FOUND'; END IF;
 structural:=structural||jsonb_build_array(jsonb_build_object('key','rules.complete','outcome',outcome,'severity','BLOCK','detail',detail));
 IF NOT EXISTS(SELECT 1 FROM finance.policy_version pv WHERE (pv.tenant_id,pv.company_id,pv.id)=(t,c,p.policy_id) AND pv.criticality_complete)
 THEN outcome:='UNKNOWN'; detail:='NO_APPROVED_CRITICAL_REGISTRY';
 ELSIF EXISTS(SELECT 1 FROM finance.policy_critical_field cf WHERE (cf.tenant_id,cf.company_id,cf.policy_id)=(t,c,p.policy_id)
 AND (SELECT count(*) FROM jsonb_array_elements((material->'fields')||(material->'financial')) fld WHERE fld->>'path'=cf.field_path AND fld->>'verified'='true' AND fld->>'value' IS NOT NULL)<>1)
 THEN outcome:='UNKNOWN'; detail:='CRITICAL_EVIDENCE_INCOMPLETE';
 ELSE outcome:='PASS'; detail:='CRITICAL_EVIDENCE_VERIFIED'; END IF;
 structural:=structural||jsonb_build_array(jsonb_build_object('key','critical.fields','outcome',outcome,'severity','BLOCK','detail',detail));
 IF finance.current_bound_reservation(t,c,p.dossier_id,p.revision_id,p.policy_id,p.dossier_version) IS NULL
 THEN outcome:='UNKNOWN'; detail:='NO_VERIFIED_CURRENT_RESERVATION'; ELSE outcome:='PASS'; detail:='SOURCE_BOUND_RESERVATION_VERIFIED'; END IF;
 structural:=structural||jsonb_build_array(jsonb_build_object('key','financial.reservation','outcome',outcome,'severity','BLOCK','detail',detail));
 -- Validate every worker result before writing any check rows.
 blocked:=EXISTS(SELECT 1 FROM jsonb_array_elements(structural) x WHERE x->>'severity'='BLOCK' AND x->>'outcome' NOT IN ('PASS','NOT_APPLICABLE'))
 OR EXISTS(SELECT 1 FROM jsonb_array_elements(results) x JOIN LATERAL(SELECT r FROM jsonb_array_elements(material->'rules') r WHERE r->>'id'=x->>'id') rr ON true
 WHERE rr.r->>'severity'='BLOCK' AND x->>'outcome' NOT IN ('PASS','NOT_APPLICABLE'));
 revision_version:=p.dossier_version;
 IF NOT blocked THEN revision_version:=revision_version+1; END IF;
 digest:=sha256(convert_to(jsonb_build_array(material,structural,results)::text,'UTF8'));
 INSERT INTO finance.check_run(tenant_id,company_id,id,dossier_id,revision_id,dossier_version,actor_id,policy_id,business_day,input_hash,blocked)
 VALUES(t,c,p.run_id,p.dossier_id,p.revision_id,revision_version,p.actor_id,p.policy_id,p.business_day,digest,blocked);
 FOR item IN SELECT value FROM jsonb_array_elements(structural) LOOP
 INSERT INTO finance.check_result(tenant_id,company_id,run_id,rule_key,outcome,severity,detail_code)
 VALUES(t,c,p.run_id,item.value->>'key',item.value->>'outcome',item.value->>'severity',item.value->>'detail');
 END LOOP;
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
    SELECT 1 FROM jsonb_array_elements((material->'fields')||(material->'financial')) fld
    CROSS JOIN LATERAL jsonb_array_elements_text(fld->'evidence') ev
    WHERE fld->>'verified'='true' AND ev=e::text))
  THEN RAISE EXCEPTION 'unbound rule evidence' USING ERRCODE='23514'; END IF;
  INSERT INTO finance.check_result(tenant_id,company_id,run_id,rule_key,outcome,severity,detail_code,rule_version,evidence_ids)
  VALUES(t,c,p.run_id,'dsl.'||(rule->>'name'),result->>'outcome',rule->>'severity',result->>'detail',(rule->>'version')::bigint,evidence);
 END LOOP;
 IF NOT blocked THEN
  UPDATE finance.dossier SET status='REVIEWABLE',version=revision_version,updated_at=GREATEST(updated_at,clock_timestamp())
  WHERE (tenant_id,company_id,id,version)=(t,c,p.dossier_id,p.dossier_version) AND status='DRAFT';
  IF NOT FOUND THEN RAISE EXCEPTION 'stale preflight dossier' USING ERRCODE='23514'; END IF;
 END IF;
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action)
 VALUES(t,c,p.run_id,p.actor_id,j,j,p.dossier_id,'SYNTHETIC_PREFLIGHT_CHECKED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
 VALUES(t,c,p.run_id,j,p.dossier_id,'SYNTHETIC_PREFLIGHT_CHECKED');
 UPDATE finance.job SET state='SUCCEEDED',lease_until=NULL,leased_by=NULL
 WHERE (tenant_id,company_id,id,fence,leased_by)=(t,c,j,f,w) AND state='LEASED' AND lease_until>clock_timestamp();
 IF NOT FOUND THEN RAISE EXCEPTION 'stale preflight lease' USING ERRCODE='40001'; END IF;
 UPDATE finance.job_attempt SET state='SUCCEEDED',finished_at=clock_timestamp()
 WHERE (tenant_id,company_id,job_id,fence,worker_id)=(t,c,j,f,w) AND state='ACTIVE';
 IF NOT FOUND THEN RAISE EXCEPTION 'stale attempt' USING ERRCODE='40001'; END IF;
 INSERT INTO finance.inbox(tenant_id,company_id,event_id,fence) VALUES(t,c,j,f);
END $$;
ALTER FUNCTION finance.complete_preflight_with_rules(uuid,uuid,uuid,bigint,uuid,jsonb,jsonb) OWNER TO finance_workflow_writer;
REVOKE ALL ON FUNCTION finance.complete_preflight_with_rules(uuid,uuid,uuid,bigint,uuid,jsonb,jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.complete_preflight_with_rules(uuid,uuid,uuid,bigint,uuid,jsonb,jsonb) TO finance_job_worker;


