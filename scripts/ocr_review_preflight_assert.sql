-- Continue after the runtime HTTP field decision over actual OCR evidence.
DO $$
DECLARE t uuid:='00000000-0000-0000-0000-000000000001';
 c uuid:='00000000-0000-0000-0000-00000000000a';
 d uuid:='00000000-0000-0000-0000-000000001f41';
 owner_id uuid:='00000000-0000-0000-0000-0000000003e9';
 accountant uuid:='00000000-0000-0000-0000-000000004001';
 check_before uuid:='00000000-0000-0000-0000-000000004010';
 check_after uuid:='00000000-0000-0000-0000-000000004011';
 hash_before bytea;hash_after bytea;
BEGIN
 IF (SELECT version FROM finance.dossier WHERE id=d)<>2
 OR (SELECT count(*) FROM finance.field_decision WHERE dossier_id=d)<>1
 OR NOT EXISTS(SELECT 1 FROM finance.field_decision WHERE dossier_id=d AND actor_id=accountant
  AND disposition='CONFIRMED_CANDIDATE' AND normalized_value='125000'
  AND cardinality(evidence_ids)>0 AND dossier_version=1)
 OR (SELECT count(*) FROM finance.audit_event WHERE resource_id=d AND action='FIELD_REVIEWED')<>1
 OR (SELECT count(*) FROM finance.outbox WHERE resource_id=d AND event_type='FIELD_REVIEWED')<>1
 OR (SELECT count(*) FROM finance.match_decision q JOIN finance.match_result m
     ON (m.tenant_id,m.company_id,m.id)=(q.tenant_id,q.company_id,q.result_id)
     WHERE m.dossier_id=d AND q.decision='CONFIRMED' AND q.dossier_version=2)<>1
 OR (SELECT count(*) FROM finance.audit_event WHERE resource_id=d AND action='MATCH_CONFIRMED')<>1
 THEN RAISE EXCEPTION 'runtime field review receipt/provenance invariant failed'; END IF;

 PERFORM set_config('finance.tenant_id',t::text,true);
 PERFORM set_config('finance.company_id',c::text,true);
 PERFORM set_config('finance.actor_id',owner_id::text,true);
 BEGIN
  PERFORM finance.prototype_preflight(t,c,d,0,owner_id,gen_random_uuid(),'2027-01-01');
  RAISE EXCEPTION 'stale preflight accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 PERFORM finance.prototype_preflight(t,c,d,2,owner_id,check_after,'2027-01-01');
 SELECT input_hash INTO hash_before FROM finance.check_run WHERE id=check_before;
 SELECT input_hash INTO hash_after FROM finance.check_run WHERE id=check_after;
 IF hash_before IS NULL OR hash_after IS NULL OR hash_before=hash_after
 OR NOT EXISTS(SELECT 1 FROM finance.check_run
    WHERE tenant_id=t AND company_id=c AND id=check_before AND dossier_id=d
      AND dossier_version=0 AND blocked)
 OR NOT EXISTS(SELECT 1 FROM finance.check_run
    WHERE tenant_id=t AND company_id=c AND id=check_after AND dossier_id=d
      AND dossier_version=2 AND blocked)
 OR NOT EXISTS(SELECT 1 FROM finance.check_result WHERE run_id=check_after
    AND rule_key='scan.release' AND outcome='UNKNOWN' AND severity='BLOCK')
 OR (SELECT status FROM finance.dossier WHERE id=d)<>'DRAFT'
 OR has_table_privilege('finance_runtime','finance.field_decision','INSERT')
 OR NOT has_function_privilege(
      'finance_runtime',
      'finance.runtime_field_decide(uuid,uuid,uuid,bigint,uuid,uuid,uuid,text,text,uuid[],uuid,uuid)',
      'EXECUTE')
 OR EXISTS(SELECT 1 FROM pg_roles WHERE rolname='finance_field_review_writer'
           AND (rolsuper OR rolbypassrls OR rolcanlogin OR rolcreaterole))
 THEN RAISE EXCEPTION 'field review invalidation/preflight invariant failed'; END IF;
 BEGIN
  UPDATE finance.field_decision SET reason='changed' WHERE dossier_id=d;
  RAISE EXCEPTION 'field decision mutated';
 EXCEPTION WHEN check_violation THEN NULL; END;
END $$;
SELECT 'PASS: actual OCR field HTTP review, immutable evidence decision, version invalidation and blocked preflight';
