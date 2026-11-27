# E18 — Workflow
Scope: Hồ sơ, review và ngoại lệ. Owner A18. Dependencies: E01, E03, E05, E11, E15, E16, E22. Owns `crates/workflow/`, `apps/web/src/features/workflow/`, `tests/workflow/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E18-R01 | Dossier lifecycle | revision, state transitions and optimistic locking, ready seal | E18-AT01: direct status PATCH denied |
| E18-R02 | Preflight | checklist, rule results, reservations, evidence and snapshot atomic | E18-AT02: stale check run or failed reservation blocks |
| E18-R03 | Review | assign queue, critical confirmation, accept/return, SoD | E18-AT03: self review or missing confirmation denied |
| E18-R04 | Exception/handoff | waiver approval, expiry, manual handoff, unknown external status handling | E18-AT04: unknown DO cannot release reservations |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E18; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
