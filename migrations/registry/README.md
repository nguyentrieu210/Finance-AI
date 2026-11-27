# Migration registry — G01 technical draft
SQLx root: `migrations/`. A00 allocates globally unique monotonically increasing UTC-style numeric IDs. Do not rewrite a migration once applied in a shared environment.

| ID | Owner | Change | Dependencies | Recovery |
|---|---|---|---|---|
| 20261127000100 | A00 | Private finance schema + vector extension | PostgreSQL 17 with pgvector available | Transaction rollback on failure; fresh dev DB discard only; shared environments use reviewed forward recovery |
| 20261127000200 | A00, A01/A02/A03/A04/A22 | Synthetic-only restricted role and 8 foundation tables | 20261127000100 | Transaction rollback; isolated test database only |
| 20261127000300 | A00/A01 | Synthetic-only durable job/attempt/inbox and worker scope | 20261127000200 | Transaction rollback; isolated test database only |
| 20261127000400 | A00/A22 | Synthetic-only audit hash chain, backfill, scoped read | 20261127000300 | Transaction rollback; isolated test DB only |
| 20261127000500 | A00/A02 | Synthetic-only read-only identity lookup role and RLS | 20261127000400 | Transaction rollback; isolated test DB only |

A synthetic-only G02 prototype adds restricted runtime role, company-scoped RLS and eight initial tables. Production login provisioning, full financial/job schema and audit chain remain unimplemented. Run only against a new test database until engineering and security review. Application roles must not own schema/tables, inherit admin, or possess BYPASSRLS. The separate finance-migrate binary embeds SQLx migrations and requires explicit --apply plus FINANCE_MIGRATION_DATABASE_URL. Never run it automatically from API/worker. The isolated integration script verifies SQLx ledger replay and checksum drift rejection. G02 is partial; see docs/g02-prototype.md.

Planned entity ownership and invariants: table-ownership.json; validated against the logical data model. Only the explicitly authorized synthetic prototype migration below is allocated; production schema remains subject to owner review. Decision register: architecture/g01-decisions.md.
