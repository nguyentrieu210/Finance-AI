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
  VALUES(t,c,doc,1,'staging/'||doc::text||'/1',clock_timestamp(),clock_timestamp()+interval '5 minutes');
  INSERT INTO finance.document_blob(tenant_id,company_id,id,generation,byte_count,sha256_digest,mime,synthetic_bytes,
   scan_state,scan_engine,scan_engine_digest,scanned_at)
  VALUES(t,c,doc,1,octet_length(payload),sha256(payload),mime,payload,'CLEAN','CLAMAV',decode(repeat('ca',32),'hex'),clock_timestamp());
  INSERT INTO finance.document_ref(tenant_id,company_id,id,dossier_id,revision_id) VALUES(t,c,doc,d,r);
 END LOOP;
END $$;
