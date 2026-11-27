# E06 — OCR
Scope: OCR và text layer. Owner A06. Dependencies: E01, E05. Owns `crates/ocr/`, `apps/web/src/features/ocr/`, `tests/ocr/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E06-R01 | Render | PDF text layer và raster per page; sandbox, cap CPU/RAM/pages | E06-AT01: malformed PDF và bomb timeout |
| E06-R02 | OCR adapter | PaddleOCR isolated; model/version, page bounding boxes, Vietnamese text | E06-AT02: scan nghiêng, chữ dấu, rotated page |
| E06-R03 | Quality | điểm chất lượng ảnh và retry preprocessing; low quality human review | E06-AT03: blur không tự xác nhận |
| E06-R04 | Caching | cache theo tenant/hash/model/config; stale job discard | E06-AT04: cache cross-tenant forbidden, model upgrade rerun |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E06; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
