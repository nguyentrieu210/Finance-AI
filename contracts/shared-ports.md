# Shared technical contracts (G01 draft)
Rust source of truth: `crates/shared/src/{ids,money,contracts}.rs`. This is a prototype interface candidate, not business or security signoff.

- Typed non-nil UUIDs, distinct tenant/company/actor/command/correlation/resource/dossier/revision/evidence IDs. Explicit tenant-global versus company scope. Command context is internal; the HTTP body cannot assert actor/scope. Authorization is checked on current verified data and same transaction as a command; matching scope alone is not permission. Resource owner/assignment/SoD rules are E03/E18.
- Money accepts only decimal strings, max 20 integer digits and 8 fractional digits consistent with NUMERIC(28,8), checked operations and currency equality. Currency syntax does not prove ISO registry validity; policy controls currency and scale, rounding and FX. No implicit conversion or rounding.
- Outcome and severity are independent typed enums; no automatic interpretation of UNKNOWN as PASS. HTTP error envelope matches draft OpenAPI Error.
- Authorization, audit and idempotency are Rust ports generic over the caller's transaction. Adapter failures do not authorize, no separate audit connection; audit record contains IDs and action only. Redacted diff schema, hash-chain and transactional outbox are E22/E01 work. Worker cannot gain business approval permissions. Ports do not constitute security implementation.
- Command/version and request canonicalization rules need engineering review before freeze. Scope and grants must be rechecked on execution. No client-supplied timestamp as authority.
- No production rule activation or bank/payment endpoints. Owner decisions for G01: finance policies/formulas/form templates, source coverage, rounding/FX/tax; security owner confirms trust boundary, AI egress and delegation; engineering owner reviews OpenAPI response headers, version semantics, schema and migration.

See http-versioning.md for the draft aggregate version token and operation-scoped immutable idempotency receipt. The Rust Version cap matches JavaScript safe integer limits.
