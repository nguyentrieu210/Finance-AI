# Verified identity profile synchronization (Migration 63)

The signed access token is verified against the pinned issuer, audience, RS256
signature, expiry and JWKS snapshot before its optional OIDC profile claims are
used. No browser-supplied name, email or subject is accepted. The existing
identity resolver must find an active identity and company membership first.

The projection updates only display_name, email and profile_issued_at for that
issuer/subject in the scoped tenant. Email must be explicitly verified. Missing
or invalid display names leave the old name in place. Tokens with an older or
equal issue timestamp cannot overwrite newer data. The identity pool does not
get membership or identity provisioning rights. RLS permits only the scoped
existing active identity; the update statement independently checks the active
company membership again. Profile data is never used as authorization.

The migration and new binary must be released together, after isolated database
tests. Verify Keycloak's finance-web client includes profile and email claims
in access tokens. Sign out and sign back in to obtain a fresh token if needed.
No user credentials, admin tokens or actual personal records belong in tests.

Acceptance: valid signed claims populate the matching profile; unverified or
missing email, older tokens, mismatched issuer/subject, inactive membership and
cross-company attempts do not update it. Non-admins must not gain access to
the member directory. Existing grant checks and financial route gates remain
unchanged. Do not activate until database and authenticated browser checks pass.
