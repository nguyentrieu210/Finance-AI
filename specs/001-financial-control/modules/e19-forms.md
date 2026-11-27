# E19 — Form
Scope: Biểu mẫu động. Owner A19. Dependencies: E01, E16. Owns `crates/forms/`, `apps/web/src/features/forms/`, `tests/forms/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E19-R01 | Form schema | typed fields, condition, source path, required and display layout | E19-AT01: invalid field dependency cycle |
| E19-R02 | Validation | server validates field schema, dynamic visibility and required fields | E19-AT02: hidden required policy inconsistency rejected |
| E19-R03 | Versioning | published form pinned to revision, migration preview only | E19-AT03: old dossier opens correct form |
| E19-R04 | Designer | admin preview, draft save, maker-checker publish via policy | E19-AT04: draft not used on active dossier |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E19; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
