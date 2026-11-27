# E17 — Knowledge
Scope: Tri thức và RAG. Owner A17. Dependencies: E01, E16. Owns `crates/knowledge/`, `apps/web/src/features/knowledge/`, `tests/knowledge/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E17-R01 | Ingestion | approved versioned clauses only, scoped chunk and embedding | E17-AT01: draft or revoked clause not retrieved for current advice |
| E17-R02 | Retrieval | tenant/company/policy version filters, lexical+vector search | E17-AT02: cross-tenant retrieval forbidden |
| E17-R03 | Explanation | cite clause and evidence, distinguish guidance from decisions | E17-AT03: hallucinated citation not shown as verified |
| E17-R04 | Change proposal | AI proposes diff or rule draft; no autonomous publish | E17-AT04: prompt injection cannot publish |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E17; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
