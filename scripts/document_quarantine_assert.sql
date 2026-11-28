-- Runs only in the disposable CI database with synthetic bytes.
DO $$
DECLARE t uuid:='00000000-0000-0000-0000-000000000001'; c uuid:='00000000-0000-0000-0000-00000000000a';
 a uuid:='00000000-0000-0000-0000-0000000003e9'; d uuid:='00000000-0000-0000-0000-000000009101';
 r uuid:='00000000-0000-0000-0000-000000009102'; i uuid:='00000000-0000-0000-0000-000000009103';
 j uuid:='00000000-0000-0000-0000-000000009104'; p bytea:=convert_to('%PDF-1.7 synthetic original','UTF8'); bad bytea;
 g bigint; count_rows bigint;
BEGIN
 PERFORM finance.prototype_reserve_upload(t,c,i,d,r,a,'synthetic.pdf',octet_length(p),sha256(p),'application/pdf');
 PERFORM finance.prototype_issue_upload_grant(t,c,i,r,a,300);
 g:=finance.prototype_issue_upload_grant(t,c,i,r,a,300);
 IF g<>2 THEN RAISE EXCEPTION 'unexpected generation'; END IF;
 BEGIN
  PERFORM finance.prototype_finalize_synthetic_upload(t,c,i,r,a,1,p);
  RAISE EXCEPTION 'stale grant accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN
  PERFORM finance.prototype_finalize_synthetic_upload(t,'00000000-0000-0000-0000-00000000000b',i,r,a,2,p);
  RAISE EXCEPTION 'cross-company finalize accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN
  PERFORM finance.prototype_finalize_synthetic_upload(t,c,i,gen_random_uuid(),a,2,p);
  RAISE EXCEPTION 'wrong revision accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN
  PERFORM finance.prototype_finalize_synthetic_upload(t,c,i,r,a,2,set_byte(p,octet_length(p)-1,0));
  RAISE EXCEPTION 'modified bytes accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 IF EXISTS(SELECT 1 FROM finance.document_blob WHERE id=i) THEN RAISE EXCEPTION 'failed finalize persisted blob'; END IF;
 IF finance.prototype_finalize_synthetic_upload(t,c,i,r,a,2,p)<>i THEN RAISE EXCEPTION 'finalize result mismatch'; END IF;
 IF finance.prototype_finalize_synthetic_upload(t,c,i,r,a,2,p)<>i THEN RAISE EXCEPTION 'replay result mismatch'; END IF;
 IF finance.prototype_upload_grant_current(t,c,i,r,a,2) THEN RAISE EXCEPTION 'finalized grant still current'; END IF;
 SELECT count(*) INTO count_rows FROM finance.document_blob WHERE id=i AND scan_state='QUARANTINED' AND synthetic_bytes=p AND sha256_digest=sha256(p);
 IF count_rows<>1 OR (SELECT count(*) FROM finance.document_ref WHERE id=i AND revision_id=r)<>1 THEN RAISE EXCEPTION 'original or ref mismatch'; END IF;
 BEGIN
  UPDATE finance.document_blob SET scan_state='QUARANTINED' WHERE id=i;
  RAISE EXCEPTION 'immutable blob update accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN
  DELETE FROM finance.document_ref WHERE id=i;
  RAISE EXCEPTION 'immutable ref delete accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 BEGIN
  PERFORM finance.prototype_cancel_upload(t,c,i,r,a);
  RAISE EXCEPTION 'finalized intent cancelled';
 EXCEPTION WHEN check_violation THEN NULL; END;
 -- Finalized bytes remain in the same dossier quota across revision IDs.
 FOR k IN 1..3 LOOP
  PERFORM finance.prototype_reserve_upload(t,c,gen_random_uuid(),d,r,a,'reserved.pdf',26214400,decode(repeat('aa',32),'hex'),'application/pdf');
 END LOOP;
 BEGIN
  PERFORM finance.prototype_reserve_upload(t,c,gen_random_uuid(),d,gen_random_uuid(),a,'overflow.pdf',26214400,decode(repeat('aa',32),'hex'),'application/pdf');
  RAISE EXCEPTION 'finalized bytes omitted from quota';
 EXCEPTION WHEN check_violation THEN NULL; END;
 PERFORM finance.prototype_reserve_upload(t,c,gen_random_uuid(),d,r,a,'remaining.pdf',26214400-octet_length(p),decode(repeat('aa',32),'hex'),'application/pdf');
 IF (SELECT sum(declared_bytes) FROM finance.upload_intent WHERE dossier_id=d AND state IN ('PENDING_UPLOAD','FINALIZED'))<>104857600 THEN RAISE EXCEPTION 'quota mismatch'; END IF;
 -- Declared PDF with matching checksum but wrong magic must still fail.
 bad:=convert_to('synthetic invalid file','UTF8');
 PERFORM finance.prototype_reserve_upload(t,c,j,gen_random_uuid(),r,a,'bad.pdf',octet_length(bad),sha256(bad),'application/pdf');
 PERFORM finance.prototype_issue_upload_grant(t,c,j,r,a,300);
 BEGIN
  PERFORM finance.prototype_finalize_synthetic_upload(t,c,j,r,a,1,bad);
  RAISE EXCEPTION 'wrong signature accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 -- Expire a valid grant using test-owner fixture only.
 j:=gen_random_uuid();
 PERFORM finance.prototype_reserve_upload(t,c,j,gen_random_uuid(),r,a,'expired.pdf',octet_length(p),sha256(p),'application/pdf');
 PERFORM finance.prototype_issue_upload_grant(t,c,j,r,a,300);
 UPDATE finance.upload_grant SET issued_at=clock_timestamp()-interval '2 seconds',expires_at=clock_timestamp()-interval '1 second' WHERE intent_id=j;
 BEGIN
  PERFORM finance.prototype_finalize_synthetic_upload(t,c,j,r,a,1,p);
  RAISE EXCEPTION 'expired grant accepted';
 EXCEPTION WHEN check_violation THEN NULL; END;
 IF EXISTS(SELECT 1 FROM finance.document_blob WHERE id=j) THEN RAISE EXCEPTION 'expired finalize persisted'; END IF;
 IF has_table_privilege('finance_runtime','finance.document_blob','SELECT')
 OR NOT has_table_privilege('finance_job_worker','finance.document_blob','SELECT')
 OR has_table_privilege('finance_job_worker','finance.document_blob','INSERT')
 OR has_table_privilege('finance_job_worker','finance.document_blob','DELETE')
 OR has_column_privilege('finance_job_worker','finance.document_blob','synthetic_bytes','UPDATE')
 OR has_table_privilege('finance_runtime','finance.document_ref','INSERT')
 OR has_function_privilege('finance_runtime','finance.prototype_finalize_synthetic_upload(uuid,uuid,uuid,uuid,uuid,bigint,bytea)','EXECUTE')
 OR EXISTS(SELECT 1 FROM pg_roles WHERE rolname='finance_job_worker'
           AND (rolsuper OR rolbypassrls OR rolcanlogin OR rolcreaterole))
 OR NOT EXISTS(SELECT 1 FROM pg_policies
               WHERE schemaname='finance' AND tablename='document_blob'
                 AND policyname='document_blob_worker_read')
 OR NOT EXISTS(SELECT 1 FROM pg_class cl
               JOIN pg_namespace ns ON ns.oid=cl.relnamespace
               WHERE ns.nspname='finance' AND cl.relname='document_blob'
                 AND cl.relforcerowsecurity) THEN
  RAISE EXCEPTION 'quarantine privilege boundary mismatch';
 END IF;
END $$;
