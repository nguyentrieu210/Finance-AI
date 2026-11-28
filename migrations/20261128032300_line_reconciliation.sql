-- E10 independent, evidence-bound, immutable many-to-many reconciliation.
CREATE TABLE finance.line_reconciliation (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,id uuid NOT NULL,dossier_id uuid NOT NULL,revision_id uuid NOT NULL,
 policy_id uuid NOT NULL,actor_id uuid NOT NULL,dossier_version bigint NOT NULL,
 request_key text NOT NULL,request_digest bytea NOT NULL CHECK(octet_length(request_digest)=32),
 material jsonb NOT NULL,result jsonb NOT NULL,created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),UNIQUE(tenant_id,company_id,actor_id,request_key),
 FOREIGN KEY(tenant_id,company_id,dossier_id) REFERENCES finance.dossier(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,revision_id) REFERENCES finance.dossier_revision(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,company_id,policy_id) REFERENCES finance.policy_version(tenant_id,company_id,id),
 FOREIGN KEY(tenant_id,actor_id) REFERENCES finance.identity_subject(tenant_id,id)
);
ALTER TABLE finance.line_reconciliation ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.line_reconciliation FORCE ROW LEVEL SECURITY;
CREATE TRIGGER line_reconciliation_immutable BEFORE UPDATE OR DELETE ON finance.line_reconciliation FOR EACH ROW EXECUTE FUNCTION finance.reject_allocation_mutation();
GRANT SELECT,INSERT ON finance.line_reconciliation TO finance_matching_writer;
GRANT SELECT ON finance.policy_version,finance.document_blob,finance.upload_intent TO finance_matching_writer;
GRANT UPDATE(status) ON finance.dossier TO finance_matching_writer;
CREATE POLICY line_reconciliation_read ON finance.line_reconciliation FOR SELECT TO finance_matching_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY line_reconciliation_insert ON finance.line_reconciliation FOR INSERT TO finance_matching_writer WITH CHECK(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND actor_id=finance.scope_actor());
CREATE POLICY line_policy_read ON finance.policy_version FOR SELECT TO finance_matching_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company() AND state='PUBLISHED');
CREATE POLICY line_blob_read ON finance.document_blob FOR SELECT TO finance_matching_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE POLICY line_upload_read ON finance.upload_intent FOR SELECT TO finance_matching_writer USING(tenant_id=finance.scope_tenant() AND company_id=finance.scope_company());
CREATE FUNCTION finance.runtime_line_reconcile(t uuid,c uuid,a uuid,d uuid,v bigint,pid uuid,body jsonb,idem text,i uuid,cmd uuid,corr uuid)
RETURNS jsonb LANGUAGE plpgsql VOLATILE SECURITY DEFINER SET search_path=pg_catalog AS $$
#variable_conflict use_column
DECLARE ds finance.dossier%ROWTYPE; rev uuid; policy jsonb; digest bytea; prior finance.line_reconciliation%ROWTYPE;
 ln jsonb; ed jsonb; src jsonb; dst jsonb; conv jsonb; all_lines jsonb; ids text[]; evidence uuid; side text; quantity numeric; unit_price numeric; value numeric;
 sq numeric; tq numeric; allocated numeric; numerator numeric; denominator numeric; tolerance numeric; used_qty numeric; used_value numeric;
 leftovers jsonb:='[]'; result jsonb; complete boolean:=true; price_difference boolean:=false;
BEGIN
 IF (t,c,a) IS DISTINCT FROM (finance.scope_tenant(),finance.scope_company(),finance.scope_actor()) OR d IS NULL OR pid IS NULL OR v IS NULL OR i IS NULL OR cmd IS NULL OR corr IS NULL
 OR idem IS NULL OR idem !~ '^[A-Za-z0-9._~-]{1,128}$' OR body IS NULL OR jsonb_typeof(body) IS DISTINCT FROM 'object' OR octet_length(body::text)>2097152
 OR jsonb_typeof(body->'sources') IS DISTINCT FROM 'array' OR jsonb_typeof(body->'targets') IS DISTINCT FROM 'array' OR jsonb_typeof(body->'allocations') IS DISTINCT FROM 'array'
 OR body->>'reason' IS NULL OR octet_length(body->>'reason') NOT BETWEEN 1 AND 2048 OR btrim(body->>'reason')=''
 OR jsonb_typeof(body->'coverage_complete') IS DISTINCT FROM 'boolean'
 THEN RAISE EXCEPTION 'invalid matching command' USING ERRCODE='22023'; END IF;
 IF jsonb_array_length(body->'sources') NOT BETWEEN 1 AND 256 OR jsonb_array_length(body->'targets') NOT BETWEEN 1 AND 256 OR jsonb_array_length(body->'allocations')>1024
 THEN RAISE EXCEPTION 'matching bounds' USING ERRCODE='22023'; END IF;
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.line.match/v1/'||t::text||'/'||c::text||'/'||a::text||'/'||idem,0));
 PERFORM pg_advisory_xact_lock(hashtextextended('finance.dossier/v1/'||t::text||'/'||c::text||'/'||d::text,0));
 SELECT * INTO ds FROM finance.dossier WHERE (tenant_id,company_id,id)=(t,c,d) FOR UPDATE;
 IF NOT FOUND OR ds.owner_id=a OR NOT EXISTS(SELECT 1 FROM finance.membership m WHERE (m.tenant_id,m.company_id,m.actor_id)=(t,c,a) AND m.role='ACCOUNTANT' AND m.active AND m.valid_from<=clock_timestamp() AND (m.valid_until IS NULL OR m.valid_until>clock_timestamp())) THEN RAISE EXCEPTION 'independent accountant required' USING ERRCODE='42501'; END IF;
 SELECT dr.id INTO STRICT rev FROM finance.dossier_revision dr WHERE (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number)=(t,c,d,ds.revision);
 IF NOT EXISTS(SELECT 1 FROM finance.review_assignment ra WHERE (ra.tenant_id,ra.company_id,ra.dossier_id,ra.revision_id,ra.accountant_id)=(t,c,d,rev,a)) THEN RAISE EXCEPTION 'assigned accountant required' USING ERRCODE='42501'; END IF;
 digest:=sha256(convert_to(jsonb_build_array(d,v,pid,body)::text,'UTF8'));
 SELECT * INTO prior FROM finance.line_reconciliation x WHERE (x.tenant_id,x.company_id,x.actor_id,x.request_key)=(t,c,a,idem);
 IF FOUND THEN IF prior.request_digest<>digest THEN RAISE EXCEPTION 'matching replay conflict' USING ERRCODE='23505'; END IF; RETURN prior.result; END IF;
 IF ds.version<>v OR ds.status NOT IN ('DRAFT','NEEDS_INPUT','REVIEWABLE') THEN RAISE EXCEPTION 'stale dossier' USING ERRCODE='23514'; END IF;
 SELECT p.finance_gate->'line_match' INTO policy FROM finance.policy_version p WHERE (p.tenant_id,p.company_id,p.id)=(t,c,pid) AND p.state='PUBLISHED' AND p.dossier_kind=ds.kind
 AND p.effective_from<=(clock_timestamp() AT TIME ZONE 'UTC')::date AND p.effective_until>(clock_timestamp() AT TIME ZONE 'UTC')::date;
 IF policy IS NULL THEN RAISE EXCEPTION 'approved line matching policy required' USING ERRCODE='23514'; END IF;
 tolerance:=(policy->>'price_tolerance')::numeric;
 all_lines:=(body->'sources')||(body->'targets');
 IF (SELECT count(DISTINCT x->>'id') FROM jsonb_array_elements(all_lines) x)<>jsonb_array_length(all_lines)
 OR (SELECT count(DISTINCT x->>'evidence_id') FROM jsonb_array_elements(all_lines) x)<>jsonb_array_length(all_lines)
 THEN RAISE EXCEPTION 'duplicate line identity or evidence' USING ERRCODE='23514'; END IF;
 FOR ln IN SELECT value FROM jsonb_array_elements(all_lines) LOOP
  IF ln->>'id' IS NULL OR octet_length(ln->>'id') NOT BETWEEN 1 AND 128 OR ln->>'item' IS NULL OR octet_length(ln->>'item') NOT BETWEEN 1 AND 128
  OR ln->>'unit' IS NULL OR octet_length(ln->>'unit') NOT BETWEEN 1 AND 32 OR ln->>'currency' IS DISTINCT FROM policy->>'currency'
  OR ln->>'quantity' IS NULL OR ln->>'quantity' !~ '^[0-9]{1,20}([.][0-9]{1,8})?$'
  OR ln->>'unit_price' IS NULL OR ln->>'unit_price' !~ '^[0-9]{1,20}([.][0-9]{1,8})?$'
  OR ln->>'value' IS NULL OR ln->>'value' !~ '^[0-9]{1,20}([.][0-9]{1,8})?$'
  THEN RAISE EXCEPTION 'invalid line operands' USING ERRCODE='23514'; END IF;
  quantity:=(ln->>'quantity')::numeric; unit_price:=(ln->>'unit_price')::numeric; value:=(ln->>'value')::numeric;
  IF quantity<=0 OR quantity*unit_price<>value THEN RAISE EXCEPTION 'line arithmetic mismatch; no implicit rounding' USING ERRCODE='23514'; END IF;
  evidence:=(ln->>'evidence_id')::uuid;
  side:=CASE WHEN EXISTS(SELECT 1 FROM jsonb_array_elements(body->'sources') x WHERE x->>'id'=ln->>'id') THEN 'SOURCE' ELSE 'TARGET' END;
  IF NOT EXISTS(SELECT 1 FROM finance.evidence e JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(e.tenant_id,e.company_id,e.ocr_run_id)
    JOIN finance.document_ref dr ON (dr.tenant_id,dr.company_id,dr.id)=(o.tenant_id,o.company_id,o.document_id)
    JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(dr.tenant_id,dr.company_id,dr.id)
    JOIN finance.upload_intent u ON (u.tenant_id,u.company_id,u.id)=(b.tenant_id,b.company_id,b.id)
    WHERE (e.tenant_id,e.company_id,e.id)=(t,c,evidence) AND (dr.dossier_id,dr.revision_id,o.revision_id)=(d,rev,rev)
    AND b.scan_state='CLEAN' AND b.generation=o.generation AND b.sha256_digest=o.source_digest AND u.state='FINALIZED' AND u.generation=b.generation
    AND (CASE WHEN side='SOURCE' THEN u.document_kind IN ('CONTRACT','CONTRACT_APPENDIX','ACCEPTANCE_RECORD') ELSE u.document_kind IN ('INVOICE','PAYMENT_REQUEST','ADVANCE_SETTLEMENT') END)
    AND NOT EXISTS(SELECT 1 FROM finance.ocr_run newer WHERE (newer.tenant_id,newer.company_id,newer.document_id,newer.revision_id)=(o.tenant_id,o.company_id,o.document_id,o.revision_id) AND (newer.created_at,newer.id)>(o.created_at,o.id)))
  THEN RAISE EXCEPTION 'line evidence stale or wrong source' USING ERRCODE='23514'; END IF;
 END LOOP;
 IF (SELECT count(DISTINCT jsonb_build_array(x->>'source',x->>'target')) FROM jsonb_array_elements(body->'allocations') x)<>jsonb_array_length(body->'allocations') THEN RAISE EXCEPTION 'duplicate allocation edge' USING ERRCODE='23514'; END IF;
 FOR ed IN SELECT value FROM jsonb_array_elements(body->'allocations') LOOP
  SELECT x INTO src FROM jsonb_array_elements(body->'sources') x WHERE x->>'id'=ed->>'source';
  SELECT x INTO dst FROM jsonb_array_elements(body->'targets') x WHERE x->>'id'=ed->>'target';
  IF src IS NULL OR dst IS NULL OR src->>'item' IS DISTINCT FROM dst->>'item' OR ed->>'source_quantity' IS NULL OR ed->>'source_quantity' !~ '^[0-9]{1,20}([.][0-9]{1,8})?$'
  OR ed->>'target_quantity' IS NULL OR ed->>'target_quantity' !~ '^[0-9]{1,20}([.][0-9]{1,8})?$' THEN RAISE EXCEPTION 'invalid matching edge' USING ERRCODE='23514'; END IF;
  sq:=(ed->>'source_quantity')::numeric; tq:=(ed->>'target_quantity')::numeric;
  numerator:=1; denominator:=1;
  IF src->>'unit'<>dst->>'unit' THEN
   SELECT x INTO conv FROM jsonb_array_elements(policy->'conversions') x WHERE x->>'from'=src->>'unit' AND x->>'to'=dst->>'unit';
   IF conv IS NULL THEN RAISE EXCEPTION 'unapproved UOM conversion' USING ERRCODE='23514'; END IF;
   numerator:=(conv->>'numerator')::numeric; denominator:=(conv->>'denominator')::numeric;
  END IF;
  IF sq<=0 OR tq<=0 OR sq*numerator<>tq*denominator THEN RAISE EXCEPTION 'quantity conversion mismatch' USING ERRCODE='23514'; END IF;
  IF abs((src->>'unit_price')::numeric*denominator-(dst->>'unit_price')::numeric*numerator)>tolerance*numerator THEN price_difference:=true; END IF;
 END LOOP;
 FOR ln IN SELECT value FROM jsonb_array_elements(all_lines) LOOP
  side:=CASE WHEN EXISTS(SELECT 1 FROM jsonb_array_elements(body->'sources') x WHERE x->>'id'=ln->>'id') THEN 'SOURCE' ELSE 'TARGET' END;
  IF side='SOURCE' THEN SELECT coalesce(sum((x->>'source_quantity')::numeric),0) INTO used_qty FROM jsonb_array_elements(body->'allocations') x WHERE x->>'source'=ln->>'id';
  ELSE SELECT coalesce(sum((x->>'target_quantity')::numeric),0) INTO used_qty FROM jsonb_array_elements(body->'allocations') x WHERE x->>'target'=ln->>'id'; END IF;
  quantity:=(ln->>'quantity')::numeric; unit_price:=(ln->>'unit_price')::numeric;
  IF used_qty>quantity THEN RAISE EXCEPTION 'line overallocated' USING ERRCODE='23514'; END IF;
  IF side='TARGET' AND used_qty<>quantity THEN complete:=false; END IF;
  leftovers:=leftovers||jsonb_build_array(jsonb_build_object('id',ln->>'id','side',side,'remaining_quantity',(quantity-used_qty)::text,'remaining_value',((quantity-used_qty)*unit_price)::text,'evidence_id',ln->>'evidence_id'));
 END LOOP;
 result:=jsonb_build_object('id',i,'dossier_version',v+1,'policy_id',pid,'outcome',CASE WHEN price_difference OR NOT complete THEN 'FAIL' WHEN body->>'coverage_complete'<>'true' THEN 'UNKNOWN' ELSE 'MATCHED' END,'residuals',leftovers,'price_difference',price_difference,'advisory_only',true);
 INSERT INTO finance.line_reconciliation(tenant_id,company_id,id,dossier_id,revision_id,policy_id,actor_id,dossier_version,request_key,request_digest,material,result)
 VALUES(t,c,i,d,rev,pid,a,v+1,idem,digest,body,result);
 UPDATE finance.dossier SET version=v+1,status='DRAFT',updated_at=GREATEST(updated_at,clock_timestamp()) WHERE (tenant_id,company_id,id)=(t,c,d);
 INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES(t,c,cmd,a,cmd,corr,d,'MATCH_CONFIRMED');
 INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES(t,c,cmd,cmd,d,'MATCH_CONFIRMED');
 RETURN result;
END $$;
ALTER FUNCTION finance.runtime_line_reconcile(uuid,uuid,uuid,uuid,bigint,uuid,jsonb,text,uuid,uuid,uuid) OWNER TO finance_matching_writer;
REVOKE ALL ON FUNCTION finance.runtime_line_reconcile(uuid,uuid,uuid,uuid,bigint,uuid,jsonb,text,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_line_reconcile(uuid,uuid,uuid,uuid,bigint,uuid,jsonb,text,uuid,uuid,uuid) TO finance_runtime;
CREATE FUNCTION finance.runtime_line_reconciliations(t uuid,c uuid,a uuid,d uuid)
RETURNS jsonb LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE data jsonb;
BEGIN
 IF NOT finance.field_access_allowed(t,c,d,a) THEN RAISE EXCEPTION 'forbidden matching read' USING ERRCODE='42501'; END IF;
 SELECT coalesce(jsonb_agg(jsonb_build_object('result',x.result,'material',x.material,'actor_id',x.actor_id,'created_at',x.created_at,'stale',x.dossier_version<>ds.version) ORDER BY x.created_at DESC),'[]'::jsonb) INTO data
 FROM (SELECT l.* FROM finance.line_reconciliation l WHERE (l.tenant_id,l.company_id,l.dossier_id)=(t,c,d) ORDER BY l.created_at DESC LIMIT 20) x
 JOIN finance.dossier ds ON (ds.tenant_id,ds.company_id,ds.id)=(t,c,d);
 RETURN data;
END $$;
ALTER FUNCTION finance.runtime_line_reconciliations(uuid,uuid,uuid,uuid) OWNER TO finance_matching_writer;
REVOKE ALL ON FUNCTION finance.runtime_line_reconciliations(uuid,uuid,uuid,uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_line_reconciliations(uuid,uuid,uuid,uuid) TO finance_runtime;
