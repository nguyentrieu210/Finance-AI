# E09 — Validation
Scope: Kiểm tra trường và nguồn. Owner A09. Dependencies: E01, E08. Owns `crates/validation/`, `apps/web/src/features/validation/`, `tests/validation/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E09-R01 | Schema validation | required types, enum, date, checksum, decimal scale | E09-AT01: invalid date, negative outside signed adjustment |
| E09-R02 | Semantic validation | document-specific fields and totals, field provenance and freshness | E09-AT02: subtotal mismatch, stale reference |
| E09-R03 | Critical field registry | policy lists critical paths and required evidence; unknown not pass | E09-AT03: missing beneficiary blocks |
| E09-R04 | Invalidation | dependency graph from document/ref changes to checks/review | E09-AT04: late extraction does not overwrite revision |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E09; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
