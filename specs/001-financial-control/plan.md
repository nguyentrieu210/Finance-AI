# Implementation Plan: Finance AI Rust-native
**Branch** 001-financial-control | **Spec** spec.md | **Status** Draft

## Summary
Modular monolith Rust with separate API and worker binaries, React SPA, PostgreSQL, private object storage, OCR and model adapters. MVP implements US1–US7, US8 only after external dependencies cleared. Manual handoff is a first-class workflow, not a mock integration.

## Technical context
- Rust stable (pin rust-toolchain.toml after compatibility spike), Cargo workspace, Axum, Tokio, Tower, SQLx, Serde, Reqwest, Utoipa, rust_decimal, tracing. DB migrations SQLx. IDs UUID; timestamps UTC. PostgreSQL NUMERIC with explicit scales. PostgreSQL-backed jobs and outbox; no additional queue framework in initial pilot. Do not require Redis or Temporal.
- React + TypeScript + Vite, Tailwind/shadcn, TanStack Query/Table, React Hook Form/Zod, Zustand only local UI, PDF.js, React Flow optional policy editor, ECharts KPI. API schemas generated or validated against OpenAPI; PWA caches static assets only, not financial data.
- OCR PaddleOCR isolated process/service (Python permitted); pdfium-render for raster/text, lopdf for PDF assembly, Calamine for XLSX, rust_xlsxwriter for output, DOCX template adapter, LibreOffice headless isolated sandbox. Evaluate licenses and document fidelity, memory limits, fonts. No arbitrary macro execution, egress disabled in converter.
- AI OpenAI or approved provider behind Rust trait; schema-bound results via Serde validation, budget/time caps, no model tools. RAG PostgreSQL + pgvector only approved policy text; citations by policy version and clause. Do not send unapproved documents outside trust boundary.
- PostgreSQL with tenant-scoped queries, RLS defense in depth, app DB role non-owner without BYPASSRLS; tenant context transaction-local. S3-compatible private storage, encryption, short signed URL, per-object authorization; local development MinIO or compatible approved server. OIDC provider Keycloak or enterprise IdP.
- Docker Compose pilot, Caddy/TLS, isolated network, Prometheus/Grafana, structured logs, CI GitHub Actions, Nextest, Playwright, k6, cargo-deny/audit. Pin dependencies, SBOM, image scan; no production secret in repository.

## Constitution check
All eight principles apply. No exception proposed. Rust domain separated from external OCR; deterministic rules and Decimal; evidence traceable; human decisions; security, versioning, tests and simple infrastructure. Any future exception requires ADR approval.

## Architecture and boundaries
```
Browser -> Caddy -> API (authz / dossier / policy / review / export / integration)
                    | PostgreSQL (transaction + outbox + jobs)
                    | Private object storage
Worker -> OCR sandbox -> extraction adapter -> candidate/evidence persistence
       -> matching + deterministic rules -> results
       -> template renderer sandbox -> package storage
       -> DO/ERP adapters (disabled until verified)
```
API never waits for OCR synchronously. Domain crates independent of HTTP and SQLx; application orchestrates ports; infrastructure implements adapters. Worker receives tenant, dossier, revision and idempotency key; compare revision before write, stale job audit and discard. Queue lease, heartbeats, backoff, dead letter and recovery. Outbox delivery at-least-once, effects idempotent.

## Repository layout (target)
- apps/api, apps/worker: Rust composition roots; apps/web: React SPA.
- crates/shared: shared DTOs, traits and error envelope owned A00; crates/core: E01 business primitives, queue and transactions. Each module crate uses the exact slug in modules/ and task graph. Shared persistence adapter conventions frozen at G01.
- services/ocr and services/converter: isolated non-Rust adapters; no business rules.
- migrations: central registry, infra: compose and monitoring, tests: module and system tests.
- UI routes: architecture/ui.md is authoritative.

## Workflow and transactions
Use normative states in architecture/workflow.md. Readiness atomically checks current revision, policy validity, completed check run, blocking results, acknowledgments, valid waivers, required confirmations and duplicate reservation. Lock relevant financial reference rows or use serializable retry for cumulative balance. Export only pinned snapshot. Handoff, review and external status separate; return creates revision and supersedes old readiness/waivers; release reservations only according to approved cancellation/reconciliation policy. Lock/unique constraints, not UI, prevent concurrent duplicate requests.

## Security model
Threats: upload malware, prompt injection, IDOR, tenant bleed, stale approval, invoice replay, forged webhook, AI data leakage, insider abuse, untrusted templates, zip bombs. Controls: deny default RBAC/ABAC and company scope, OIDC issuer/aud/exp validation, CSRF protection for cookie auth, strict CORS, no token localStorage, tenant RLS tests, scan/quarantine, rate limits, egress allowlist, sandbox CPU/memory/time, audit, evidence hashes, signatures and replay windows for integrations. Key rotation, backup encrypted, retention and deletion policy approval. AI confidence not verification. Logs mask sensitive fields; audit detail restricted.

## Phases
G00 bootstrap, G01 contracts, G02 foundation integration, G03 vertical slice, G04 pilot, G05 live integration. Tasks execute as a DAG, not a sequential wave. See tasks.md. Each user story ends with independent E2E; full merge gate reruns suite. No production cutover before finance/security acceptance.

## V2 amendments
See architecture and module specs for authoritative v2 boundaries. Audit/authorization/transaction primitives are foundation, not later retrofit. PostgreSQL queue + outbox (no second job platform initially). Contracts are draft until G01; all externally dependent functions disabled by default. Run scripts/validate_spec.py before handoff. No tests or deploy implied by this document.
