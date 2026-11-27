# Research / architectural decisions
All decisions proposed; compatibility spikes are tasks, not completed benchmarks.

| ADR | Decision | Alternatives rejected / rationale | Validation |
|---|---|---|---|
| 001 | Axum/Tokio + SQLx | Python/Node core: not aligned Rust-first; Actix viable but choose Axum ecosystem | Cargo build, integration tests |
| 002 | Decimal / NUMERIC | IEEE floats unsuitable for exact monetary policy | precision, overflow, rounding fixtures |
| 003 | Modular monolith | Microservices add transactions/deployment overhead | load benchmark, module dependency check |
| 004 | PostgreSQL queue/outbox | Kafka/Temporal/Redis unnecessary initially | lease recovery, dedupe race, retry |
| 005 | React SPA | Rust WASM UI and Next SSR add complexity | PDF viewer, accessibility spike |
| 006 | OCR isolated service | Pure Rust OCR rewrite high risk; ONNX optional after benchmark | Vietnamese documents, layout/table recall |
| 007 | Provider adapter + structured extraction | LLM free text or LLM approval unsafe | schema, injection, cost, fidelity eval |
| 008 | Typed JSON rule AST | Arbitrary JavaScript/Rhai scripts risk and nondeterminism | compiler, type checking, rule fixtures |
| 009 | Immutable snapshots | mutable evidence undermines audit | revision invalidation tests |
| 010 | Manual handoff + real adapter gated | Fake DO integration misleading | verified handoff and sandbox contract |
| 011 | OIDC + RLS + service auth | homegrown passwords and UI-only role checks unsafe | cross-tenant/SoD regression |
| 012 | S3 compatible + signed URLs | DB blobs and public URLs unsuitable | quarantine/access/restore tests |
| 013 | LibreOffice sandbox | untested Rust DOCX renderer fidelity risk | approved template golden compare |

## Decisions requiring business owner
- D01: exact dossier variants, required forms and policy source/version; map sample rules to approved clauses.
- D02: approval authority, SoD, overrideable vs hard block, critical fields, tolerance, rounding and currency/FX.
- D03: reference source ownership, contract change/credit note treatment, duplicate identity, reservations and freshness.
- D04: advance deadline, settlement linkage, partial settlement, refund and extra payment conditions.
- D05: DO/ERP API capability, integration credentials, status semantics, webhooks and sandbox.
- D06: AI provider approval, residency, data retention, security threat review, budget.
- D07: pilot cohort, baseline, benchmark labels, recovery and performance targets.

Until D01–D07 resolved, synthetic testing and draft policy only; do not present example policy as approved. No production send to AI, DO, ERP or payment.

## Official references
- Spec Kit: https://github.github.com/spec-kit/quickstart
- Axum: https://docs.rs/axum/ ; SQLx: https://docs.rs/sqlx/ ; Decimal: https://docs.rs/rust_decimal/
- PostgreSQL RLS: https://www.postgresql.org/docs/current/ddl-rowsecurity.html
- PaddleOCR: https://github.com/PaddlePaddle/PaddleOCR
- OpenAI Structured Outputs: https://platform.openai.com/docs/guides/structured-outputs
Versions/licenses/API details to verify during G00/G01; links are implementation references, not evidence of executed tests.

## V2 amendments
See architecture and module specs for authoritative v2 boundaries. Audit/authorization/transaction primitives are foundation, not later retrofit. PostgreSQL queue + outbox (no second job platform initially). Contracts are draft until G01; all externally dependent functions disabled by default. Run scripts/validate_spec.py before handoff. No tests or deploy implied by this document.
