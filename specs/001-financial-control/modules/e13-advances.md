# E13 — Advance
Scope: Tạm ứng và quyết toán. Owner A13. Dependencies: E01, E11. Owns `crates/advances/`, `apps/web/src/features/advances/`, `tests/advances/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E13-R01 | Advance issuance | purpose, due date, employee/vendor, amount, verified external issue | E13-AT01: draft not counted as issued |
| E13-R02 | Settlement allocation | multiple advances, partial application, same currency unless approved FX | E13-AT02: concurrent settlement against one advance |
| E13-R03 | Refund and extra | refund confirmed from source; extra payment separate reservation | E13-AT03: unverified refund not consumed |
| E13-R04 | Overdue | as-of business date, extensions approved, overdue warning/block policy | E13-AT04: timezone boundary and extension |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E13; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
