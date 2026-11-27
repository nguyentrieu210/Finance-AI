# System gates

These gates are intentionally synthetic and run on a single Git commit.

- **G02 technical evidence:** the `verify` job runs restricted PostgreSQL identity, permission-revocation, audit-chain and durable job fencing tests.
- **G03 technical evidence:** the same `verify` job runs the three-kind dossier/API/browser slice, financial reservation race, sealed snapshot and fenced export completion; sibling jobs exercise S3 capability, Vietnamese OCR, PaddleOCR, PDF rendering, image quality and office export.
- **G04 technical evidence:** `restore-drill` performs a real PostgreSQL `pg_dump`/`pg_restore` drill and validates the restored SQLx ledger, table inventory, pgvector extension, fixture and audit chain. `system-gates` succeeds only when every listed job succeeds on the same commit.

This is not finance/security signoff and is not production authorization. Representative business corpus evaluation and external DO/ERP sandbox evidence remain separate release dependencies.
