# Pilot runtime

The API binary has an explicit `--pilot` mode that mounts authenticated business routes against restricted PostgreSQL roles. The no-argument mode remains health-only and fail-closed.

Required deployment inputs: `FINANCE_RUNTIME_DATABASE_URL`, `FINANCE_IDENTITY_DATABASE_URL`, `FINANCE_TENANT_ID`, `FINANCE_COMPANY_ID`, `FINANCE_OIDC_ISSUER`, `FINANCE_OIDC_AUDIENCE`, `FINANCE_OIDC_JWKS_FILE`, and a 32-byte hex `FINANCE_CURSOR_KEY_HEX`. Database login roles must only be able to assume `finance_runtime` or `finance_identity_lookup`; superuser, BYPASSRLS, CREATEDB and CREATEROLE sessions are rejected.

The JWKS file is bounded, parsed through the RS256 access-token verifier and refreshed atomically. A bad refresh never replaces the last valid verifier. Readiness turns unhealthy when the last trusted snapshot expires or either restricted database pool is unavailable.

This mode does not embed provider credentials, auto-provision identities, choose tenant/company from requests, or enable live DO/ERP connectors. Those remain external deployment/integration dependencies.
