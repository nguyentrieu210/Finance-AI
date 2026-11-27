# E20 — Template
Scope: Xuất DOCX/XLSX/PDF. Owner A20. Dependencies: E01, E05, E18, E19. Owns `crates/templates/`, `apps/web/src/features/templates/`, `tests/templates/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E20-R01 | Template registry | version, mapping, sandbox, macros forbidden, approved fonts | E20-AT01: malicious DOCX and unmapped placeholder rejected |
| E20-R02 | Render | Decimal formatting, tables, pagination, deterministic input manifest | E20-AT02: Vietnamese fonts, multi-page table, conversion fail |
| E20-R03 | Package | ordered PDF + originals index, evidence manifest, hashes, signed access | E20-AT03: same idempotency key returns same package |
| E20-R04 | Export job | atomic success only after storage verify; retry cleanup and stale protection | E20-AT04: storage failure no EXPORTED |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E20; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
