# E12 — Contract
Scope: Hợp đồng và công nợ tham chiếu. Owner A12. Dependencies: E01, E11. Owns `crates/contracts/`, `apps/web/src/features/contracts/`, `tests/contracts/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E12-R01 | Contract versions | base/addendum effective range, approved adjustments, obligations | E12-AT01: future addendum not used |
| E12-R02 | Acceptance allocations | receipt/acceptance lines and billed/paid allocations across dossiers | E12-AT02: same goods received twice claim |
| E12-R03 | Reference provenance | immutable verified import, as_of, completeness; no live ERP assumption | E12-AT03: stale or incomplete snapshot unknown |
| E12-R04 | Outstanding | per contract/installment balances, retention and advance deduction | E12-AT04: partial installments, retention release policy |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E12; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
