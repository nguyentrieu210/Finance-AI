# E07 — Document Intelligence
Scope: Layout, phân loại, bảng biểu. Owner A07. Dependencies: E01, E06. Owns `crates/document-intelligence/`, `apps/web/src/features/document-intelligence/`, `tests/document-intelligence/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E07-R01 | Classification | kind candidates, unknown/ambiguous, user confirm; multi-doc split | E07-AT01: PDF chứa hợp đồng và phụ lục |
| E07-R02 | Layout | blocks, reading order, tables, merged cells, continuation pages | E07-AT02: table across pages |
| E07-R03 | Normalization | giữ raw Unicode, dấu số, ngày, timezone; ambiguity explicit | E07-AT03: 1.234 vs 1,234 không đoán |
| E07-R04 | Provenance | block IDs link original page; no invented cells | E07-AT04: missing table cell stays unknown |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E07; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
