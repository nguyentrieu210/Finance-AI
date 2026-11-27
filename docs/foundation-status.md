# Foundation implementation
G00 implementation candidate. Liveness API, fail-closed readiness, disabled worker, React shell, PostgreSQL development configuration and CI baseline only. No financial operation, identity, upload, storage, OCR or integration endpoint is enabled. Do not use for real dossiers. Rust 1.85.1 selected as an initial compatibility pin; review supported security versions and dependency advisories before pilot. PostgreSQL image tag is a development pin; review digest and hardening at G02. Storage/OIDC compose profiles, real integration checks, nextest, Playwright and security tooling are future gate work.

Local:
- Rust: `cargo build --locked --workspace`, `cargo test --locked --workspace`, `cargo run -p finance-api`
- Web: `cd apps/web && npm ci && npm run dev`
- DB config: `FINANCE_DEV_DB_PASSWORD=<local-only> docker compose -f infra/compose.yaml config --quiet`; start only in an isolated dev environment.
- Spec: `python3 -m venv .venv && .venv/bin/pip install -r scripts/spec-requirements.txt && .venv/bin/python scripts/validate_spec.py`

G00 remains unchecked until a real CI run passes. G01 remains draft pending finance, security and engineering review, OpenAPI lint, typed client and migration registry. No scope reduction.

## Local evidence (candidate)
- Rust container 1.85.1: cargo generate-lockfile, fmt, clippy with -D warnings, test (3 API tests passed), build (all workspace packages): passed.
- Node 22.23.5/npm 10.9.9: npm ci --include=dev, typecheck, source-shell test (1 passed), build, npm audit: passed. This is not browser E2E.
- Compose config: passed; no database started or tested.
- Spec validator: passed, 153 tasks, 110 acceptance descriptions, 90 API operations. OpenAPI structural validation: passed. TypeScript API generation and --check: passed. No live business routes implemented.
- No production deploy. Real CI run remains a G00 condition; finance/security approvals and technical freeze remain G01 conditions.
