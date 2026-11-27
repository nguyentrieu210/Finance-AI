# E02 — Identity
Scope: OIDC và phiên đăng nhập. Owner A02. Dependencies: E01. Owns `crates/identity/`, `apps/web/src/features/identity/`, `tests/identity/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E02-R01 | Token validation | issuer/audience/algorithm/expiry/JWKS allowlist; fail closed | E02-AT01: token hết hạn, sai issuer, rotated key |
| E02-R02 | Membership | mapping subject nội bộ, phiên và revoke; không tự gia nhập tenant | E02-AT02: subject không có membership bị từ chối |
| E02-R03 | Service identity | worker identity scoped, không impersonate approver | E02-AT03: service không gọi approval |
| E02-R04 | Session | logout, revoke và refresh theo IdP; không lưu token trong localStorage | E02-AT04: phiên bị revoke không mutate |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E02; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
