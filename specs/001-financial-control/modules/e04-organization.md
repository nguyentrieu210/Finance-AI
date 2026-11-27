# E04 — Organization
Scope: Tenant, công ty, đơn vị. Owner A04. Dependencies: E01, E03. Owns `crates/organization/`, `apps/web/src/features/organization/`, `tests/organization/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E04-R01 | Hierarchy | tenant/company/department/cost center, unique code và effective dates | E04-AT01: cross-company FK reject |
| E04-R02 | Membership admin | mời/gán/thu hồi role với audit, admin kỹ thuật không mặc định đọc hồ sơ | E04-AT02: privilege escalation denied |
| E04-R03 | Company settings | locale, timezone, currency, retention, provider egress policy versioned | E04-AT03: change timezone không sửa lịch sử |
| E04-R04 | Scope resolution | cost center và company hợp lệ tại ngày nghiệp vụ | E04-AT04: disabled org cannot create dossier |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E04; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
