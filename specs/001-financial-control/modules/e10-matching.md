# E10 — Matching
Scope: Đối chiếu chéo. Owner A10. Dependencies: E01, E09. Owns `crates/matching/`, `apps/web/src/features/matching/`, `tests/matching/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E10-R01 | Identity match | exact normalized invoice/vendor/contract identity; fuzzy suggestions only | E10-AT01: different issuer same invoice number not same identity |
| E10-R02 | Line match | many-to-many allocation, UOM conversion approved, unmatched rows explicit | E10-AT02: split receipt, incompatible UOM |
| E10-R03 | Reconciliation | line quantity, price, totals, tolerance pinned; no silent rounding | E10-AT03: quantity mismatch, rounding boundary |
| E10-R04 | Explain | operands and evidence pairs, manual matching reason and reviewer | E10-AT04: ambiguous match remains unknown |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E10; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
