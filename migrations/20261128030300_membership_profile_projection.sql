-- Optional verified identity display metadata. Never used as authorization credentials.
ALTER TABLE finance.identity_subject
 ADD COLUMN display_name text CHECK(display_name IS NULL OR length(display_name) BETWEEN 1 AND 200),
 ADD COLUMN email text CHECK(email IS NULL OR length(email) BETWEEN 1 AND 320);
CREATE FUNCTION finance.runtime_membership_profiles(
 t uuid,c uuid,a uuid,after_id uuid,only_id uuid,n integer
) RETURNS TABLE(
 id uuid,subject_id uuid,company_id uuid,roles text[],active boolean,version bigint,
 display_name text,email text
)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT m.id,m.subject_id,m.company_id,m.roles,m.active,m.version,i.display_name,i.email
 FROM finance.runtime_memberships(t,c,a,after_id,only_id,n) m
 JOIN finance.identity_subject i ON i.tenant_id=t AND i.id=m.subject_id
 ORDER BY m.id
$$;
ALTER FUNCTION finance.runtime_membership_profiles(uuid,uuid,uuid,uuid,uuid,integer)
 OWNER TO finance_membership_writer;
REVOKE ALL ON FUNCTION finance.runtime_membership_profiles(uuid,uuid,uuid,uuid,uuid,integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION finance.runtime_membership_profiles(uuid,uuid,uuid,uuid,uuid,integer) TO finance_runtime;
