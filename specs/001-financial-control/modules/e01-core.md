# E01 — Core
Scope: Kiểu dữ liệu và hợp đồng chung. Owner A01. Dependencies: foundation only. Owns `crates/core/`, `apps/web/src/features/core/`, `tests/core/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E01-R01 | Money/Quantity | Decimal chính xác, scale và currency/UOM rõ; reject overflow, không float | E01-AT01: VND/USD, khác currency, overflow, decimal serialization |
| E01-R02 | Command envelope | CommandId, tenant/company, actor, expected version, correlation; UTC clock injectable | E01-AT02: request thiếu actor, version conflict, clock frozen |
| E01-R03 | Durable jobs | Lease có fencing token, heartbeat, retry bounded, dead letter; worker stale không commit | E01-AT03: crash, lease expiry, duplicate delivery |
| E01-R04 | Transaction ports | Transaction scope, idempotency, inbox/outbox atomic; retry conflict bounded | E01-AT04: rollback không mất event, replay khác payload 409 |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E01; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
