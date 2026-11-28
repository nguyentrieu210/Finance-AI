-- Verified OIDC profile projection only: no account provisioning or membership changes.
ALTER TABLE finance.identity_subject
 ADD COLUMN profile_issued_at bigint CHECK(profile_issued_at IS NULL OR profile_issued_at>=0);
GRANT UPDATE(display_name,email,profile_issued_at) ON finance.identity_subject TO finance_identity_lookup;
CREATE POLICY identity_lookup_profile_update ON finance.identity_subject
 FOR UPDATE TO finance_identity_lookup
 USING (tenant_id=finance.scope_tenant() AND active
  AND issuer=NULLIF(current_setting('finance.identity_issuer',true),'')
  AND subject=NULLIF(current_setting('finance.identity_subject',true),'')
)
 WITH CHECK (tenant_id=finance.scope_tenant() AND active
  AND issuer=NULLIF(current_setting('finance.identity_issuer',true),'')
  AND subject=NULLIF(current_setting('finance.identity_subject',true),''));
