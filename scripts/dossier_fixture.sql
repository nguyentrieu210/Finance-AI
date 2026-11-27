-- Run after earlier revocation tests; restore synthetic requester only in disposable DB.
UPDATE finance.membership SET active=true, valid_until=NULL WHERE tenant_id='00000000-0000-0000-0000-000000000001' AND company_id='00000000-0000-0000-0000-00000000000a' AND actor_id='00000000-0000-0000-0000-0000000003e9' AND role='REQUESTER';
INSERT INTO finance.identity_subject(tenant_id,id,issuer,subject) VALUES ('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-0000000003ec','https://synthetic.invalid','second-requester');
INSERT INTO finance.membership(tenant_id,company_id,actor_id,role) VALUES ('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-0000000003ec','REQUESTER');

-- CLEAN template sources for E20 API tests. Bytes are synthetic and remain inside disposable PostgreSQL.
INSERT INTO finance.dossier(tenant_id,company_id,id,owner_id,code,kind)
VALUES('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a',
'00000000-0000-0000-0000-000000001a00','00000000-0000-0000-0000-0000000003f0','SYN-TEMPLATE-SOURCE','PAYMENT');
INSERT INTO finance.dossier_revision(tenant_id,company_id,dossier_id,id,owner_id,number)
VALUES('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a',
'00000000-0000-0000-0000-000000001a00','00000000-0000-0000-0000-000000001a01',
'00000000-0000-0000-0000-0000000003f0',0);

DO $$
DECLARE t uuid:='00000000-0000-0000-0000-000000000001';
 c uuid:='00000000-0000-0000-0000-00000000000a';
 d uuid:='00000000-0000-0000-0000-000000001a00';
 r uuid:='00000000-0000-0000-0000-000000001a01';
 a uuid:='00000000-0000-0000-0000-0000000003f0';
 doc uuid; payload bytea; mime text; name text; n integer:=0;
BEGIN
 FOREACH doc IN ARRAY ARRAY[
  '00000000-0000-0000-0000-000000001a02'::uuid,
  '00000000-0000-0000-0000-000000001a03'::uuid,
  '00000000-0000-0000-0000-000000001a04'::uuid
 ] LOOP
  n:=n+1;
  IF n=1 THEN payload:=convert_to('%PDF-1.4'||chr(10)||'synthetic template v1','UTF8');mime:='application/pdf';name:='template-v1.pdf';
  ELSIF n=2 THEN payload:=decode('504b0304','hex')||convert_to('vbaProject.bin synthetic macro','UTF8');mime:='application/vnd.openxmlformats-officedocument.wordprocessingml.document';name:='macro.docx';
  ELSE payload:=convert_to('%PDF-1.4'||chr(10)||'synthetic template v2','UTF8');mime:='application/pdf';name:='template-v2.pdf'; END IF;
  INSERT INTO finance.upload_intent(tenant_id,company_id,id,dossier_id,revision_id,actor_id,filename,
   declared_bytes,declared_sha256,declared_mime,state,generation,document_kind)
  VALUES(t,c,doc,d,r,a,name,octet_length(payload),sha256(payload),mime,'FINALIZED',1,'TEMPLATE_SOURCE');
  INSERT INTO finance.upload_grant(tenant_id,company_id,intent_id,generation,staging_key,issued_at,expires_at)
  VALUES(t,c,doc,1,'staging/'||doc::text||'/1',statement_timestamp(),statement_timestamp()+interval '5 minutes');
  INSERT INTO finance.document_blob(tenant_id,company_id,id,generation,byte_count,sha256_digest,mime,synthetic_bytes,
   scan_state,scan_engine,scan_engine_digest,scanned_at)
  VALUES(t,c,doc,1,octet_length(payload),sha256(payload),mime,payload,'CLEAN','CLAMAV',decode(repeat('ca',32),'hex'),clock_timestamp());
  INSERT INTO finance.document_ref(tenant_id,company_id,id,dossier_id,revision_id) VALUES(t,c,doc,d,r);
 END LOOP;
END $$;

-- E18 READY fixture: REVIEWABLE dossier with current PASS checks, published assets and active hold.
INSERT INTO finance.dossier(tenant_id,company_id,id,owner_id,code,kind,status,version,revision)
VALUES('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a',
'00000000-0000-0000-0000-000000001800','00000000-0000-0000-0000-0000000003e9',
'SYN-READY','SETTLEMENT','REVIEWABLE',0,0);
INSERT INTO finance.dossier_revision(tenant_id,company_id,dossier_id,id,owner_id,number)
VALUES('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a',
'00000000-0000-0000-0000-000000001800','00000000-0000-0000-0000-000000001801',
'00000000-0000-0000-0000-0000000003e9',0);
INSERT INTO finance.check_run(
 tenant_id,company_id,id,dossier_id,revision_id,dossier_version,actor_id,policy_id,business_day,input_hash,blocked)
VALUES('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a',
'00000000-0000-0000-0000-000000001802','00000000-0000-0000-0000-000000001800',
'00000000-0000-0000-0000-000000001801',0,'00000000-0000-0000-0000-0000000003e9',
'00000000-0000-0000-0000-000000001600','2027-01-01',decode(repeat('18',32),'hex'),false);
INSERT INTO finance.check_result(tenant_id,company_id,run_id,rule_key,outcome,severity,detail_code)
VALUES('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a',
'00000000-0000-0000-0000-000000001802','ready.synthetic','PASS','BLOCK','SYNTHETIC_PASS');
INSERT INTO finance.form_version(
 tenant_id,company_id,id,form_id,policy_id,name,version,state,schema_json,author_id,checker_id,published_at)
VALUES('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a',
'00000000-0000-0000-0000-000000001803','00000000-0000-0000-0000-000000001803',
'00000000-0000-0000-0000-000000001600','Synthetic settlement form',1,'PUBLISHED',
'{"version":1,"fields":[{"key":"amount","type":"money","required":true,"required_when_visible":false,"visible_if":null,"options":null,"source_path":"settlement.amount","layout":{"row":0,"column":0,"width":12}}]}'::jsonb,
'00000000-0000-0000-0000-0000000003e8','00000000-0000-0000-0000-0000000003e9',clock_timestamp());
INSERT INTO finance.template_version(
 tenant_id,company_id,id,template_id,policy_id,name,version,state,format,source_digest,author_id,checker_id,published_at)
VALUES('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a',
'00000000-0000-0000-0000-000000001804','00000000-0000-0000-0000-000000001804',
'00000000-0000-0000-0000-000000001600','Synthetic settlement package',1,'PUBLISHED','PDF',
decode(repeat('1a',32),'hex'),'00000000-0000-0000-0000-0000000003e8',
'00000000-0000-0000-0000-0000000003e9',clock_timestamp());
INSERT INTO finance.reservation(
 tenant_id,company_id,id,revision_id,currency,amount,resource_ids,state,remaining)
VALUES('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a',
'00000000-0000-0000-0000-000000001805','00000000-0000-0000-0000-000000001801',
'VND',10,ARRAY['00000000-0000-0000-0000-000000001210'::uuid],'RESERVED',10);
INSERT INTO finance.reservation_line(tenant_id,company_id,reservation_id,resource_id,amount)
VALUES('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a',
'00000000-0000-0000-0000-000000001805','00000000-0000-0000-0000-000000001210',10);

-- E23 manual bridge fixture. These packages are synthetic READY receipts used only by integration tests.
INSERT INTO finance.dossier(tenant_id,company_id,id,owner_id,code,kind,status,version,revision)
VALUES
('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000002300','00000000-0000-0000-0000-0000000003e9','SYN-HANDOFF','SETTLEMENT','EXPORTED',0,0),
('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000002310','00000000-0000-0000-0000-0000000003e9','SYN-RECON','SETTLEMENT','HANDED_OFF',0,0);
INSERT INTO finance.dossier_revision(tenant_id,company_id,dossier_id,id,owner_id,number) VALUES
('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000002300','00000000-0000-0000-0000-000000002301','00000000-0000-0000-0000-0000000003e9',0),
('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000002310','00000000-0000-0000-0000-000000002311','00000000-0000-0000-0000-0000000003e9',0);
INSERT INTO finance.dossier_snapshot(tenant_id,company_id,id,dossier_id,revision_id,policy_id,input_hash,canonical_manifest_hash,template_version_id) VALUES
('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000002303','00000000-0000-0000-0000-000000002300','00000000-0000-0000-0000-000000002301','00000000-0000-0000-0000-000000001600',decode(repeat('23',32),'hex'),decode(repeat('24',32),'hex'),'00000000-0000-0000-0000-000000001804'),
('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000002313','00000000-0000-0000-0000-000000002310','00000000-0000-0000-0000-000000002311','00000000-0000-0000-0000-000000001600',decode(repeat('25',32),'hex'),decode(repeat('26',32),'hex'),'00000000-0000-0000-0000-000000001804');
INSERT INTO finance.package(tenant_id,company_id,id,snapshot_id,template_id,state,manifest_hash) VALUES
('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000002304','00000000-0000-0000-0000-000000002303','00000000-0000-0000-0000-000000001804','READY',decode(repeat('27',32),'hex')),
('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000002314','00000000-0000-0000-0000-000000002313','00000000-0000-0000-0000-000000001804','READY',decode(repeat('28',32),'hex'));
INSERT INTO finance.handoff(tenant_id,company_id,id,snapshot_id,package_id,channel,state,external_ref,evidence_ids)
VALUES('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000002315','00000000-0000-0000-0000-000000002313','00000000-0000-0000-0000-000000002314','MANUAL','UNKNOWN','MANUAL-UNKNOWN-1',ARRAY[]::uuid[]);
INSERT INTO finance.reconciliation_case(tenant_id,company_id,id,handoff_id,state,reason,channel,evidence_ids)
VALUES('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000002316','00000000-0000-0000-0000-000000002315','OPEN','Manual acknowledgement requires authoritative reconciliation','DO',ARRAY[]::uuid[]);
