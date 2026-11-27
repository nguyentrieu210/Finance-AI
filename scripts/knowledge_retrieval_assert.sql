DO $$
DECLARE t uuid:='00000000-0000-0000-0000-000000000001';c uuid:='00000000-0000-0000-0000-00000000000a';
 maker uuid:='00000000-0000-0000-0000-000000001501';checker uuid:='00000000-0000-0000-0000-000000001502';
 p uuid:=gen_random_uuid();cl uuid:=gen_random_uuid();cl2 uuid:=gen_random_uuid();first uuid:=gen_random_uuid();second uuid:=gen_random_uuid();
 h bytea:=decode(repeat('22',32),'hex');m bytea:=decode(repeat('33',32),'hex');result uuid;
BEGIN
 INSERT INTO finance.policy_version(tenant_id,company_id,id,dossier_kind,author_id,effective_from,effective_until,source_digest)
 VALUES(t,c,p,'ADVANCE',maker,'2027-01-01','2028-01-01',h);
 INSERT INTO finance.policy_clause VALUES(t,c,p,cl,'K1','Synthetic advance required',h,1),(t,c,p,cl2,'Synthetic settlement receipt',h,2);
 INSERT INTO finance.checklist_item VALUES(t,c,p,gen_random_uuid(),cl,'ADVANCE_REQUEST',true);
 BEGIN
  PERFORM finance.prototype_index_clause(t,c,p,cl,first,0,0,26,m,'[1,0,0]'::finance.vector);
  RAISE EXCEPTION 'draft indexed';
 EXCEPTION WHEN check_violation THEN NULL; END;
 PERFORM finance.prototype_policy_transition(t,c,p,maker,'IN_REVIEW');
 PERFORM finance.prototype_policy_transition(t,c,p,checker,'PUBLISHED');
 PERFORM finance.prototype_index_clause(t,c,p,cl,first,0,0,char_length('Synthetic advance required'),m,'[1,0,0]'::finance.vector);
 PERFORM finance.prototype_index_clause(t,c,p,cl2,second,0,0,char_length('Synthetic settlement receipt'),m,'[0,1,0]'::finance.vector);
 SELECT chunk_id INTO result FROM finance.prototype_retrieve_knowledge(t,c,'ADVANCE','2027-06-01','advance',m,'[0,1,0]'::finance.vector,1);
 IF result IS DISTINCT FROM first THEN RAISE EXCEPTION 'lexical priority wrong'; END IF;
 SELECT chunk_id INTO result FROM finance.prototype_retrieve_knowledge(t,c,'ADVANCE','2027-06-01','unmatched',m,'[0,1,0]'::finance.vector,1);
 IF result IS DISTINCT FROM second THEN RAISE EXCEPTION 'vector ranking wrong'; END IF;
 IF EXISTS(SELECT 1 FROM finance.prototype_retrieve_knowledge(t,'00000000-0000-0000-0000-00000000000b','ADVANCE','2027-06-01','advance',m,'[1,0,0]'::finance.vector,20))
 OR EXISTS(SELECT 1 FROM finance.prototype_retrieve_knowledge(t,c,'ADVANCE','2027-06-01','advance',h,'[1,0,0]'::finance.vector,20))
 OR EXISTS(SELECT 1 FROM finance.prototype_retrieve_knowledge(t,c,'ADVANCE','2028-01-01','advance',m,'[1,0,0]'::finance.vector,20))
 THEN RAISE EXCEPTION 'retrieval scope or version leak'; END IF;
 PERFORM finance.prototype_policy_transition(t,c,p,checker,'REVOKED');
 IF EXISTS(SELECT 1 FROM finance.prototype_retrieve_knowledge(t,c,'ADVANCE','2027-06-01','advance',m,'[1,0,0]'::finance.vector,20))
 THEN RAISE EXCEPTION 'revoked policy retrieved'; END IF;
 IF has_table_privilege('finance_runtime','finance.knowledge_chunk','SELECT') THEN RAISE EXCEPTION 'fixture knowledge exposed'; END IF;
END $$;
SELECT 'PASS: actual pgvector ranking, lexical priority, approved clause source, scope/model/date isolation and revocation';
