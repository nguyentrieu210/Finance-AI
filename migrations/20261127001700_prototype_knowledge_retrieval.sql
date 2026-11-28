-- E17 synthetic pgvector retrieval, no production embedding provider or AI publication.
-- Fixture dimension 3 is deliberately not the production model dimension.
CREATE TABLE finance.knowledge_chunk (
 tenant_id uuid NOT NULL,company_id uuid NOT NULL,policy_id uuid NOT NULL,clause_id uuid NOT NULL,id uuid NOT NULL,
 ordinal integer NOT NULL CHECK(ordinal>=0 AND ordinal<128),
 source_start integer NOT NULL CHECK(source_start>=0),source_end integer NOT NULL CHECK(source_end>source_start),
 excerpt text NOT NULL CHECK(octet_length(excerpt) BETWEEN 1 AND 8192),
 model_digest bytea NOT NULL CHECK(octet_length(model_digest)=32),
 embedding finance.vector(3) NOT NULL CHECK(finance.vector_norm(embedding)>0),
 created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 PRIMARY KEY(tenant_id,company_id,id),
 UNIQUE(tenant_id,company_id,policy_id,clause_id,model_digest,ordinal),
 FOREIGN KEY(tenant_id,company_id,policy_id,clause_id) REFERENCES finance.policy_clause(tenant_id,company_id,policy_id,id)
);
ALTER TABLE finance.knowledge_chunk ENABLE ROW LEVEL SECURITY;
ALTER TABLE finance.knowledge_chunk FORCE ROW LEVEL SECURITY;
CREATE TRIGGER knowledge_immutable BEFORE UPDATE OR DELETE ON finance.knowledge_chunk FOR EACH ROW EXECUTE FUNCTION finance.reject_synthetic_document_mutation();
CREATE FUNCTION finance.prototype_index_clause(t uuid,c uuid,p uuid,cl uuid,i uuid,ord integer,first_char integer,last_char integer,m bytea,v finance.vector)
RETURNS uuid LANGUAGE plpgsql VOLATILE SET search_path=pg_catalog AS $$
DECLARE policy finance.policy_version%ROWTYPE; source text; snippet text;
BEGIN
 IF current_setting('transaction_isolation')<>'read committed' OR first_char IS NULL OR last_char IS NULL
 OR first_char<0 OR last_char<=first_char OR m IS NULL OR octet_length(m)<>32
 OR v IS NULL OR finance.vector_dims(v)<>3 OR finance.vector_norm(v)<=0
 THEN RAISE EXCEPTION 'invalid chunk' USING ERRCODE='23514'; END IF;
 SELECT * INTO policy FROM finance.policy_version WHERE tenant_id=t AND company_id=c AND id=p FOR SHARE;
 IF NOT FOUND OR policy.state<>'PUBLISHED' THEN RAISE EXCEPTION 'unapproved policy' USING ERRCODE='23514'; END IF;
 SELECT excerpt INTO source FROM finance.policy_clause WHERE tenant_id=t AND company_id=c AND policy_id=p AND id=cl;
 IF NOT FOUND OR last_char>char_length(source) THEN RAISE EXCEPTION 'invalid clause locator' USING ERRCODE='23514'; END IF;
 snippet:=substring(source FROM first_char+1 FOR last_char-first_char);
 INSERT INTO finance.knowledge_chunk(tenant_id,company_id,policy_id,clause_id,id,ordinal,source_start,source_end,excerpt,model_digest,embedding)
 VALUES(t,c,p,cl,i,ord,first_char,last_char,snippet,m,v);
 RETURN i;
END $$;
-- Scope, effective policy and model are explicit. Vector distance ranks only
-- approved source chunks; lexical hits rank first. No text generation.
CREATE FUNCTION finance.prototype_retrieve_knowledge(t uuid,c uuid,k text,day date,q text,m bytea,v finance.vector,n integer)
RETURNS TABLE(chunk_id uuid,policy_id uuid,clause_id uuid,excerpt text,source_digest bytea,source_page integer,distance double precision)
LANGUAGE plpgsql STABLE SET search_path=pg_catalog AS $$
DECLARE p uuid;
BEGIN
 IF q IS NULL OR octet_length(q) NOT BETWEEN 1 AND 256 OR btrim(q)=''
 OR n IS NULL OR n NOT BETWEEN 1 AND 20 OR m IS NULL OR octet_length(m)<>32
 OR v IS NULL OR finance.vector_dims(v)<>3 OR finance.vector_norm(v)<=0
 THEN RAISE EXCEPTION 'invalid retrieval request' USING ERRCODE='23514'; END IF;
 p:=finance.prototype_effective_policy(t,c,k,day);
 IF p IS NULL THEN RETURN; END IF;
 RETURN QUERY SELECT x.id,x.policy_id,x.clause_id,x.excerpt,cl.source_digest,cl.source_page,
 (x.embedding OPERATOR(finance.<=>) v)
 FROM finance.knowledge_chunk x JOIN finance.policy_clause cl
 ON (cl.tenant_id,cl.company_id,cl.policy_id,cl.id)=(x.tenant_id,x.company_id,x.policy_id,x.clause_id)
 WHERE x.tenant_id=t AND x.company_id=c AND x.policy_id=p AND x.model_digest=m
 ORDER BY (position(lower(q) in lower(x.excerpt))>0) DESC,(x.embedding OPERATOR(finance.<=>) v),x.id LIMIT n;
END $$;
REVOKE ALL ON FUNCTION finance.prototype_index_clause(uuid,uuid,uuid,uuid,uuid,integer,integer,integer,bytea,finance.vector),
 finance.prototype_retrieve_knowledge(uuid,uuid,text,date,text,bytea,finance.vector,integer) FROM PUBLIC;
