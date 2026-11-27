# E22 — Audit
Scope: Audit và integrity. Owner A22. Dependencies: E01. Owns `crates/audit/`, `apps/web/src/features/audit/`, `tests/audit/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E22-R01 | Atomic events | mutation event same transaction, actor, scope, correlation, revision, redacted diff | E22-AT01: rollback leaves neither effect nor event |
| E22-R02 | Tamper evidence | append-only role, per-tenant sequence/hash chain, external checkpoint optional required prod | E22-AT02: tamper detection and delete denied |
| E22-R03 | Read/export | scoped auditor query and export, access event, no raw secret | E22-AT03: auditor cannot read other company |
| E22-R04 | Retention | legal hold and approved deletion with tombstone, backup restore verify chain | E22-AT04: restore integrity and retention conflict |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E22; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
