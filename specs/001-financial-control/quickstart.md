# Quickstart (target commands, not executed)
After implementing G00/G01, install pinned Rust toolchain, Node LTS, Docker Compose, Git and Specify CLI. Configure `.env.example` with test credentials, provider disabled by default. Never commit secrets or upload real financial docs without authorization.

```sh
docker compose up -d postgres storage oidc
cargo sqlx migrate run
cargo run -p finance-api
# separate shell
cargo run -p finance-worker
cd apps/web && npm ci && npm run dev
```
Expected: local login, synthetic dossier upload, review extracted fields, preflight, export, manual handoff and accountant accept/return. Fixture provider explicitly labeled synthetic; production mode cannot enable fixture provider.

## Proposed CI commands
```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --workspace --all-features
cargo deny check
cargo audit
cd apps/web && npm ci && npm run lint && npm run typecheck && npm run test && npm run build
npx playwright test
```
Configure dedicated integration jobs with real PostgreSQL/object storage, OCR smoke and provider sandbox tests when credentials approved. Run security, migration, restore, performance and AI benchmark as release gates. Commands/package names are implementation targets, not working scripts in this spec-only bundle.

## V2 amendments
See architecture and module specs for authoritative v2 boundaries. Audit/authorization/transaction primitives are foundation, not later retrofit. PostgreSQL queue + outbox (no second job platform initially). Contracts are draft until G01; all externally dependent functions disabled by default. Run scripts/validate_spec.py before handoff. No tests or deploy implied by this document.
