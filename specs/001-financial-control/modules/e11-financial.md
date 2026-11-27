# E11 — Financial
Scope: Tính toán, allocation, reservations. Owner A11. Dependencies: E01, E22. Owns `crates/financial/`, `apps/web/src/features/financial/`, `tests/financial/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E11-R01 | Money arithmetic | gross/net/tax/discount/retention/FX explicitly typed per policy | E11-AT01: currency mix, tax rounding, signed adjustment |
| E11-R02 | Allocation ledger | append-only effects, unique external transaction, reversal link; no double counting | E11-AT02: duplicate paid event, partial payment reversal |
| E11-R03 | Reservation | atomic lock order, idempotent reserve/commit/consume/release, reconcile uncertainty | E11-AT03: two concurrent requests exceed cap; timeout no release |
| E11-R04 | Payable calculation | cap min of applicable verified constraints; no negative balance silently clamped | E11-AT04: missing history unknown, negative remaining flagged |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E11; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
