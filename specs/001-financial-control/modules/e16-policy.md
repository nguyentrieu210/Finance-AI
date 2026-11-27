# E16 — Policy
Scope: Quy định và governance. Owner A16. Dependencies: E01, E03, E15, E22. Owns `crates/policy/`, `apps/web/src/features/policy/`, `tests/policy/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E16-R01 | Policy lifecycle | draft/review/publish/retire/revoke; maker-checker; immutable versions | E16-AT01: overlap active scopes rejected |
| E16-R02 | Clause mapping | rule/checklist/form/workflow links to approved source clause | E16-AT02: unmapped rule cannot publish |
| E16-R03 | Effective resolution | company + kind + effective business date, priority explicit; conflict blocks | E16-AT03: ambiguous effective version |
| E16-R04 | Impact | diff, affected dossiers, re-evaluate open revisions on required change | E16-AT04: revoked version blocks new submission |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E16; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
