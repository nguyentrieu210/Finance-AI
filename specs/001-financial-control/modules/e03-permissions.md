# E03 — Permission
Scope: Authorization và SoD. Owner A03. Dependencies: E01, E02. Owns `crates/permissions/`, `apps/web/src/features/permissions/`, `tests/permissions/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E03-R01 | Resource access | RBAC + ABAC, company, ownership, assigned reviewer, deny by default | E03-AT01: IDOR, cùng tenant khác company |
| E03-R02 | RLS | non-owner non-BYPASSRLS, FORCE RLS khi phù hợp, SET LOCAL trong transaction | E03-AT02: pool reuse không rò tenant |
| E03-R03 | SoD | người lập, người yêu cầu ngoại lệ không tự phê duyệt; actor độc lập | E03-AT03: dual-role self approval denied |
| E03-R04 | Delegation | quyền có thời hạn, scope và revoke, kiểm tra tại command execution | E03-AT04: delegation hết hạn và stale browser |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E03; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
