# E08 — Extraction
Scope: AI Gateway và structured extraction. Owner A08. Dependencies: E01, E07, E22. Owns `crates/extraction/`, `apps/web/src/features/extraction/`, `tests/extraction/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E08-R01 | Provider policy | approved provider/region, redaction, token cap, timeout, no tools | E08-AT01: unapproved provider blocked |
| E08-R02 | Typed candidates | schema theo document kind, raw+normalized+evidence, model/prompt version | E08-AT02: schema violation rejected |
| E08-R03 | Verification | candidate, correction, verified separate; critical independent review | E08-AT03: high confidence không auto verified |
| E08-R04 | Evaluation | golden corpus per field/kind, cost latency and false acceptance | E08-AT04: regression, injection text treated as data |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E08; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
