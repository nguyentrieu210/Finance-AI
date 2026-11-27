# G02 foundation prototype — synthetic only
Scope authorized by repository owner to continue technical prototype work using synthetic data. G01 finance/security/engineering owner decisions remain OPEN, no production authorization. This isolated branch does not mark any G02 or E01–E04/E22 task DONE.

## Implemented
- Migration 20261127000200 creates 8 real tables: tenant, company, identity_subject, membership, organization_unit, audit_event, outbox, idempotency_record. Composite tenant/company keys, scoped foreign keys, forced RLS and no-login non-superuser/no-BYPASSRLS runtime role. No production login provisioned.
- finance-core Rust crate: transaction-local scope via set_config(..., true); reject privileged current/session roles; prototype internal create_unit command checks active org-admin membership, tenant/company/identity validity, then persists unit + audit event + outbox event + idempotency receipt atomically. Advisory lock serializes same scoped operation key; canonical length-delimited SHA-256; conflict on changed input. No business HTTP routes enabled.
- Real isolated PG17.6 integration fixture (two tenants, three companies, four synthetic subjects), actual nonprivileged test login, restricted role, parallel same-key retry, unauthorized requester/expired role/cross-scope commands, rollback on audit conflict, pool reuse and scoped FK/RLS rejection. SQL verifies exactly two successful mutations, two audit/outbox/idempotency rows, no runtime UPDATE/DELETE on audit or INSERT on membership.
- G00/G01 previous CI evidence retained. Prototype CI adds real DB test. Existing SQLx baseline test updated to assert two migrations and eight tables.

## Limitations
This is an internal prototype, not a secure public API. Custom PostgreSQL scope GUCs assume trusted application code and are not protection against arbitrary SQL by a compromised runtime. Only org creation command checks membership; raw scoped transaction is internal infrastructure, not an authorization decision. Direct runtime inserts are trusted-code privileges. Concurrent role revocation/TOCTOU, OIDC/JWKS, session revoke, delegation/SoD and full ABAC need implementation. Audit append-only privileges are tested; per-tenant hash chain, read/export, external anchor and retention not implemented. Outbox persistence only, no dispatch/inbox/job lease/fencing. No complete organization settings/hierarchy lifecycle or membership management. No payment or document endpoints, real data, AI provider, deployment or merge.

## VPS evidence
Rust fmt/clippy/build and 14 ordinary tests PASS; separately executed PostgreSQL integration test 1/1 PASS with actual restricted login; SQL assertions PASS; baseline replay and checksum drift rejection PASS. No claim of full module acceptance.
