# E24 — Analytics
Scope: KPI và báo cáo. Owner A24. Dependencies: E01, E18, E22. Owns `crates/analytics/`, `apps/web/src/features/analytics/`, `tests/analytics/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E24-R01 | KPI definitions | first-pass accepted, active time, rule coverage denominator and excluded unknown | E24-AT01: returned/reopened case counted once |
| E24-R02 | Scoped queries | tenant/company filters, time zone, pagination, aggregates | E24-AT02: no cross-company aggregate leak |
| E24-R03 | Operational dashboards | queue age, errors, cost, stale references and reservations | E24-AT03: dead-letter alarm and unknown state |
| E24-R04 | Export | authorized report, masked PII, audit and reproducible filter | E24-AT04: CSV formula injection and unauthorized export |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E24; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
