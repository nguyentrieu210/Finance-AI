# Migration registry — G01 technical draft
SQLx root: `migrations/`. A00 allocates globally unique monotonically increasing UTC-style numeric IDs. Do not rewrite a migration once applied in a shared environment.

| ID | Owner | Change | Dependencies | Recovery |
|---|---|---|---|---|
| 20261127000100 | A00 | Private finance schema + vector extension | PostgreSQL 17 with pgvector available | Transaction rollback on failure; fresh dev DB discard only; shared environments use reviewed forward recovery |

Application and worker roles, tenant scope, RLS, finance/audit/job tables are G02/engine work, not enabled. Run only against a new test database until engineering and security review. Application roles must not own schema/tables, inherit admin, or possess BYPASSRLS. SQLx migrator invocation remains G02 wiring; raw SQL baseline is validated in an isolated test database.
