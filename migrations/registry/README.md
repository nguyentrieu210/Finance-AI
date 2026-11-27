# Migration registry — G01 technical draft
SQLx root: `migrations/`. A00 allocates globally unique monotonically increasing UTC-style numeric IDs. Do not rewrite a migration once applied in a shared environment.

| ID | Owner | Change | Dependencies | Recovery |
|---|---|---|---|---|
| 20261127000100 | A00 | Private finance schema + vector extension | PostgreSQL 17 with pgvector available | Transaction rollback on failure; fresh dev DB discard only; shared environments use reviewed forward recovery |

Application and worker roles, tenant scope, RLS, finance/audit/job tables are G02/engine work, not enabled. Run only against a new test database until engineering and security review. Application roles must not own schema/tables, inherit admin, or possess BYPASSRLS. The separate finance-migrate binary embeds SQLx migrations and requires explicit --apply plus FINANCE_MIGRATION_DATABASE_URL. Never run it automatically from API/worker. The isolated integration script verifies SQLx ledger replay and checksum drift rejection. G02 role/transaction implementation remains open.

Planned entity ownership and invariants: table-ownership.json; validated against the logical data model. No business migration IDs allocated until dependencies and owner review are satisfied. Decision register: architecture/g01-decisions.md.
