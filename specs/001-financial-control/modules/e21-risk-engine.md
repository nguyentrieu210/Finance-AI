# E21 — Risk
Scope: Ngoại lệ và rủi ro. Owner A21. Dependencies: E01, E10, E15, E18. Owns `crates/risk-engine/`, `apps/web/src/features/risk-engine/`, `tests/risk-engine/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E21-R01 | Signals | duplicate/near match, vendor bank changes, anomalous amounts; advisory | E21-AT01: LLM signal cannot auto approve or reject |
| E21-R02 | Case review | assign and resolve signal with reason, evidence, owner | E21-AT02: unauthorized closure rejected |
| E21-R03 | Waiver tracking | link immutable finding and approval, expiry/revocation; no override nonwaivable | E21-AT03: changed revision invalidates waiver |
| E21-R04 | Escalation | severity and routing by approved policy, no automatic transfer | E21-AT04: unresolved blocking signal prevents ready |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E21; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
