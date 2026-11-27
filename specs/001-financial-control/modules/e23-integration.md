# E23 — Integration
Scope: DO/ERP adapter và đối soát. Owner A23. Dependencies: E01, E11, E18, E22. Owns `crates/integration/`, `apps/web/src/features/integration/`, `tests/integration/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E23-R01 | Manual bridge | reference import verified by maker-checker; manual handoff with receipt | E23-AT01: manual submitted not approved |
| E23-R02 | DO adapter | capability discovery, external idempotency, status mapping, signed webhook or polling | E23-AT02: timeout unknown, duplicate/out-of-order callback |
| E23-R03 | ERP adapter | read-only snapshots and settlement/payment events, freshness/completeness | E23-AT03: duplicate transaction, stale import |
| E23-R04 | Reconciliation | unknown external state queue, authoritative lookup, authorized correction; no transfer endpoint | E23-AT04: lost ack and reversal, no auto-release |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E23; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
