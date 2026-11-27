# G01 contract preparation — technical draft
G00 candidate at commit 180bd8f82f941f9345d2e997e2877ced9ae6cc85 passed GitHub Actions run 45926074696 (Spec/OpenAPI/Rust/frontend/Compose). PR #1 remains open; no merge claimed.
G01 candidate implements shared technical types and ports, executable schema baseline and registry. Existing OpenAPI and generated TS types remain draft and unchanged. No accounting policy signed, no security owner signoff, no auth or audit adapter, no G02 business table. G01 remains unchecked; do not unlock dependent implementation tasks as approved. Prototype interface review and missing-input list in contracts/shared-ports.md.

## Candidate evidence (VPS1 isolated worktree)
- cargo fmt --all -- --check; cargo clippy --locked --workspace --all-targets -- -D warnings; cargo test --locked --workspace (8 tests: 3 API + 5 shared); cargo build --locked --workspace: PASS on Rust 1.85.1.
- scripts/test_migration_baseline.sh: PASS, real isolated PostgreSQL 17.6 / vector 0.8.0, private finance schema denied to unprivileged role, no business tables. No host port or persistent volume, no production DB touched. CI includes this script.
- Spec validator, OpenAPI structural validator, Compose config, git diff --check and bash syntax: PASS. Existing generated frontend types unchanged.
- Draft shared ports are not working adapters. G01 not DONE: owner review, final API header/filter/version contracts, business schemas, tax/rounding/FX/source decisions, SQLx adapter wiring and security signoff remain open. G00 PR is still open; no merge or production deployment.
