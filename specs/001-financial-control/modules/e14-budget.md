# E14 — Budget
Scope: Ngân sách. Owner A14. Dependencies: E01, E11. Owns `crates/budget/`, `apps/web/src/features/budget/`, `tests/budget/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E14-R01 | Budget references | period, account/cost center, original/revised budget version | E14-AT01: wrong period or stale data |
| E14-R02 | Commitments | budget reservation and actual reconciled using disjoint buckets | E14-AT02: ERP actual already includes local commitment |
| E14-R03 | Cap | atomic all-or-none across relevant budgets and contracts | E14-AT03: multi-resource reservation rollback |
| E14-R04 | Exceptions | budget override only if policy permits and authorized | E14-AT04: nonwaivable cap blocked |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E14; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
