# E05 — Document
Scope: Chứng từ, revision và evidence. Owner A05. Dependencies: E01, E03, E22. Owns `crates/documents/`, `apps/web/src/features/documents/`, `tests/documents/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E05-R01 | Upload | MIME sniff, SHA256, quarantine, AV, quotas và object key scoped | E05-AT01: polyglot, malware, oversized rejected |
| E05-R02 | Immutable originals | upload finalize xác nhận checksum; replace tạo ref/revision, không ghi đè | E05-AT02: seal rồi thay file không sửa bản gốc |
| E05-R03 | Evidence | locator page/cell/bbox chuẩn tọa độ, hash original, excerpt; download scoped | E05-AT03: viewer mở đúng trang, cross-tenant denied |
| E05-R04 | Lifecycle | classify, supersede, retention hold và delete chỉ theo policy | E05-AT04: legal hold không xóa, orphan cleanup |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E05; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
