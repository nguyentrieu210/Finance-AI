-- Bind a fixed independent accountant to the actual OCR revision and create the pre-review preflight.
DO $$
DECLARE t uuid:='00000000-0000-0000-0000-000000000001';
 c uuid:='00000000-0000-0000-0000-00000000000a';
 d uuid:='00000000-0000-0000-0000-000000001f41';
 rev uuid:='00000000-0000-0000-0000-000000001f42';
 owner_id uuid:='00000000-0000-0000-0000-0000000003e9';
 accountant uuid:='00000000-0000-0000-0000-000000004001';
 checker uuid:='00000000-0000-0000-0000-000000004002';
 check_before uuid:='00000000-0000-0000-0000-000000004010';
BEGIN
 INSERT INTO finance.identity_subject(tenant_id,id,issuer,subject) VALUES
 (t,accountant,'https://synthetic.invalid','field-accountant'),
 (t,checker,'https://synthetic.invalid','field-checker');
 INSERT INTO finance.membership(tenant_id,company_id,actor_id,role) VALUES
 (t,c,accountant,'ACCOUNTANT'),(t,c,checker,'FINANCE_APPROVER');
 PERFORM finance.prototype_assign_candidate_review(t,c,d,rev,accountant,checker);
 PERFORM set_config('finance.tenant_id',t::text,true);
 PERFORM set_config('finance.company_id',c::text,true);
 PERFORM set_config('finance.actor_id',owner_id::text,true);
 PERFORM finance.prototype_preflight(t,c,d,0,owner_id,check_before,'2027-01-01');
 INSERT INTO finance.dossier_snapshot(
  tenant_id,company_id,id,dossier_id,revision_id,input_hash,canonical_manifest_hash
 ) VALUES(t,c,'00000000-0000-0000-0000-000000004020',d,rev,decode(repeat('40',32),'hex'),decode(repeat('41',32),'hex'));
 INSERT INTO finance.financial_resource(
  tenant_id,company_id,id,currency,cap,consumed,coverage_complete,fresh_until
 ) VALUES(t,c,'00000000-0000-0000-0000-000000004021','VND',200000,0,true,now()+interval '1 day');
 PERFORM finance.prototype_reserve_finance(
  t,c,'00000000-0000-0000-0000-000000004022',rev,'VND',125000,
  ARRAY['00000000-0000-0000-0000-000000004021'::uuid]
 );
 IF NOT EXISTS(SELECT 1 FROM finance.field_identity fi JOIN finance.extracted_field f
   ON (f.tenant_id,f.company_id,f.run_id,f.field_path)=(fi.tenant_id,fi.company_id,fi.run_id,fi.field_path)
   WHERE fi.tenant_id=t AND fi.company_id=c AND f.field_path='invoice.total'
    AND f.state='CANDIDATE' AND f.normalized_value='125000')
 THEN RAISE EXCEPTION 'actual OCR field identity missing'; END IF;
END $$;
SELECT 'PASS: actual OCR revision assigned to independent accountant and baseline preflight sealed';
