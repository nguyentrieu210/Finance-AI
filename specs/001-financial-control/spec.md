# Feature Specification: Financial dossier control platform
**Feature branch** `001-financial-control` | **Status** Draft | **Date** 2026-11-27

## Scope and outcome
MVP gồm 3 loại hồ sơ: PAYMENT (thanh toán), ADVANCE (tạm ứng), SETTLEMENT (quyết toán tạm ứng). Quy trình 6 bước nguyên bản được giữ nguyên. Nguồn lịch sử có thể nhập snapshot kế toán đã xác minh; DO xuất file thủ công khi API chưa có. Không tạo bút toán chính thức, không chuyển tiền, không giả định đã tích hợp DO/ERP. Quản lý hợp đồng/công nợ/ngân sách tối thiểu ở dạng dữ liệu tham chiếu có nguồn; toàn bộ ERP, procurement, fraud ML, SaaS billing là roadmap ngoài MVP.

## Actors
Requester: tạo và sửa bản nháp; Accountant: xác minh và review; Finance approver: phê duyệt ngoại lệ/kiểm soát theo quyền; Policy admin: cấu hình và trình publish; Auditor: đọc evidence và audit theo phạm vi; System worker: xử lý job không có quyền phê duyệt. Admin kỹ thuật không mặc nhiên có quyền xem tài chính.

## User stories / acceptance scenarios
### US1 — Khởi tạo và nhận diện (P1)
Người lập chọn đơn vị, loại hồ sơ, upload PDF/JPG/PNG/DOCX/XLSX, theo dõi trạng thái xử lý, xem dữ liệu và chứng từ cạnh nhau, xác nhận hoặc sửa giá trị. Test độc lập: Given bộ scan hợp đồng và đề nghị, when upload hợp lệ, then hệ thống trích xuất dữ liệu có evidence, không tự gán giá trị khi không đọc được. Given file không an toàn, then quarantine và không gửi OCR.
### US2 — Đối chiếu chéo (P1)
Người lập thấy sai số hợp đồng, số tiền, số lượng, đơn giá, nhà cung cấp. Test: Given hợp đồng 150 triệu, đã trả 50 triệu, đề nghị 110 triệu, then FAIL khi nguồn lịch sử được xác minh; nếu lịch sử thiếu thì UNKNOWN. Dữ liệu thay đổi làm kết quả cũ stale; rerun bắt buộc.
### US3 — Hướng dẫn hoàn thiện (P1)
Người lập thấy checklist theo loại hồ sơ và điều kiện, template đúng version, trường bắt buộc, nguồn quy định. Test: Given thanh toán mua hàng cần nghiệm thu theo policy đã publish, when thiếu nghiệm thu, then checklist missing và không thể ready. Không áp checklist mẫu chưa duyệt vào production.
### US4 — Preflight control (P1)
Người lập chạy toàn bộ rule và xem PASS/FAIL/UNKNOWN/ERROR/NOT_APPLICABLE, nguồn đối chiếu, severity và cách khắc phục. Test: Given rule bắt buộc UNKNOWN, then không thể READY; Given ngoại lệ được cho phép và người có quyền xác nhận hợp lệ, then có thể ready với audit và trạng thái gốc không bị xóa.
### US5 — Xuất và bàn giao (P1)
Người lập tạo PDF package, biểu mẫu, phụ lục, checklist, manifest và snapshot có hash. Test: Given revision READY, when export hai lần cùng idempotency key, then chỉ một export logic; sửa chứng từ tạo revision khác, package cũ giữ nguyên. Manual DO handoff yêu cầu người dùng ghi reference/biên nhận, không giả lập API thành công.
### US6 — Kế toán kiểm soát (P1)
Kế toán xem bảng kiểm tra, mở evidence, xác nhận các trường trọng yếu, xử lý ngoại lệ và quyết định accept/return. Test: Given người lập cũng là người có quyền kế toán, then không được tự accept hồ sơ mình lập. Khi return phải nêu lý do; khi accept ghi actor, version, timestamp. Trạng thái thanh toán chỉ từ ERP/đối soát xác nhận, không kích hoạt chuyển tiền.
### US7 — Quản trị policy và kiểm toán (P1)
Policy admin soạn rule, checklist, workflow/template; finance owner publish có hiệu lực; auditor truy xuất lịch sử theo thời điểm. Test: policy mới không làm thay đổi kết quả lịch sử; dossier đang mở phải được đánh giá lại nếu version bị thu hồi hoặc bắt buộc cập nhật.
### US8 — Tích hợp thực (P2, gated)
Kết nối DO/ERP qua adapter khi có API, credentials, sandbox và owner. Test: retry không tạo submission trùng, timeout chuyển trạng thái UNKNOWN/pending reconciliation; webhook sai chữ ký bị từ chối; ERP stale không cho kết luận PASS.

## Functional requirements
- FR-001: Đăng nhập OIDC, tenant/company membership, resource permission server-side, tách vai trò và kiểm tra SoD theo actor thực.
- FR-002: Tạo dossier theo 3 loại, mã duy nhất theo tenant, revision và optimistic concurrency; danh sách/lọc/tìm kiếm theo quyền.
- FR-003: Upload MIME sniff, checksum SHA-256, quarantine, malware scan, giới hạn cấu hình, immutable original; chỉ file sạch được đọc.
- FR-004: Job OCR/classify/extract async, progress, retry có giới hạn, trạng thái failed rõ ràng; worker crash khôi phục được.
- FR-005: Extract field typed, raw + normalized + evidence locator + confidence + provider/model/schema version; lưu candidate và correction riêng.
- FR-006: Viewer side-by-side, xác nhận/correction field, audit trước/sau; tài liệu thay đổi invalidate downstream.
- FR-007: Matching theo contract/invoice/PO/vendor/currency/line items; fuzzy match chỉ gợi ý, không PASS tự động.
- FR-008: Decimal math cho tiền, thuế, số lượng, tổng và số dư; quy tắc rounding/tolerance theo policy version.
- FR-009: Tham chiếu hợp đồng, vendor, công nợ, tạm ứng, budget, lịch sử hóa đơn; nguồn, thời điểm, freshness và trạng thái xác minh bắt buộc.
- FR-010: Phát hiện invoice/dossier trùng exact + near duplicate; atomic reservation khi ready và reconciliation với ERP; không đủ history => UNKNOWN.
- FR-011: Checklist động theo loại hồ sơ/điều kiện và template version; hiển thị missing/invalid/unverified.
- FR-012: Policy DSL typed allowlist, không arbitrary code; validate schema/type, preview test fixture, maker-checker, version, effective period.
- FR-013: Rule execution deterministic trên snapshot; ghi result, operands, evidence, reason, rule/version và input hash.
- FR-014: Mandatory fail/unknown/error chặn readiness; FAIL với severity WARNING cần acknowledge nếu policy yêu cầu; NOT_APPLICABLE chỉ khi applicability false có căn cứ.
- FR-015: Exception request/decision có scope rule + revision + expiry + reason + evidence, permission và SoD; không override non-waivable.
- FR-016: Workflow state machine và transition guards; atomic version check, snapshot pin, audit/outbox; không nhảy trạng thái bằng API trực tiếp.
- FR-017: Export template DOCX/XLSX/PDF theo version; merge attachments, index, manifest, checksum; lỗi render không xuất package giả thành công.
- FR-018: Handoff DO manual có reference/evidence; adapter API gated; trạng thái phân biệt generated, handed off, acknowledged, approved.
- FR-019: Accountant review dashboard, critical fields confirmation, accept/return, reason và audit; review chỉ trên snapshot được bàn giao.
- FR-020: Payment status read-only từ nguồn ERP hoặc xác nhận thủ công có evidence và quyền; không có endpoint chuyển tiền.
- FR-021: Append-only audit cho mọi mutation/decision, correlation id, actor, version, redacted diff; auditor export theo quyền.
- FR-022: Search/filter theo tenant, role, status, date; KPI first-pass, turnaround, coverage, return rate; không expose dữ liệu ngoài scope.
- FR-023: Cấu hình tenant/company, policy, locale, currency, retention; mọi reference data import có owner và provenance.
- FR-024: Adapter AI provider, structured schema validation, data minimization, timeout, cost limits, prompt injection isolation; provider failure không PASS.
- FR-025: Outbox/inbox, idempotency, reconciliation, dead-letter visibility, retry policy và observability cho integration.
- FR-026: Bảo vệ download/export, signed URL ngắn hạn, file access audit, quyền theo revision và tenant.
- FR-027: Thanh toán từng phần, nhiều hóa đơn và nhiều đợt tạm ứng; không double count; settlement partial, hoàn tiền hoặc chi bổ sung theo policy.
- FR-028: RAG chỉ giải thích theo văn bản policy đã duyệt, dẫn source/version; không thay Rule Engine hoặc tự tạo quy định.

## Edge cases
Scan mờ, bảng nhiều trang, tài liệu có nhiều currency, dấu phân cách số tiếng Việt, hóa đơn điều chỉnh/hủy, phụ lục thay giá trị hợp đồng, nhiều khoản thanh toán đồng thời, trùng hóa đơn khác định dạng, người dùng mất quyền giữa phiên, job trả muộn sau sửa revision, policy hết hiệu lực, upload có prompt injection, duplicate webhook, DO timeout, ERP dữ liệu stale, rounding chênh lệch, chữ ký scan không chứng minh xác thực. Không suy luận chữ ký thật từ hình ảnh.

## Non-functional requirements (proposed gates)
- NFR-001: Không cross-tenant access trong test suite; TLS, encryption at rest, scoped credentials, no secrets in logs.
- NFR-002: API read p95 <= 500 ms, mutation không gồm file/AI p95 <= 1 s với 50 concurrent users, 10k dossiers/tenant; đo tại staging.
- NFR-003: OCR+extract p95 <= 120 s cho hồ sơ <= 20 trang, cấu hình 4 worker, provider/fixture được ghi rõ. Không chặn HTTP chờ AI.
- NFR-004: Core rule evaluate p95 <= 2 s với 100 rules/dossier và data đã local; 100% result có version và provenance.
- NFR-005: RPO <= 24 h, RTO <= 4 h mục tiêu pilot; restore drill có bằng chứng; production targets phải được owner duyệt.
- NFR-006: UI tiếng Việt, keyboard accessible, không chỉ dựa vào màu; hướng tới WCAG 2.2 AA với audit riêng.
- NFR-007: Audit critical action coverage 100%, không mất event khi retry; cảnh báo dead-letter, queue age và provider errors.
- NFR-008: Upload mặc định 25 MiB/file, 100 MiB/dossier, 200 pages/dossier; configurable. Kiểm tra decompression bomb và malicious macro/embedded link; không thực thi macro.
- NFR-009: Bộ benchmark >= 100 dossier đã gán nhãn, stratified đủ 3 loại; field exact match >= 95% trên critical fields, công bố theo loại tài liệu; không false PASS trên blocker cases trong release corpus (không phải bảo đảm tuyệt đối thực tế).

## Success criteria (pilot targets, not measured)
SC-001: 100% dossier đã bàn giao có snapshot, manifest, rule evidence và audit. SC-002: Giảm median active handling time >= 50% so baseline same case mix. SC-003: First-pass acceptance >= 90%; theo dõi denominator và lý do return. SC-004: >= 70% applicable criteria được đánh giá tự động có evidence, không tính UNKNOWN. SC-005: 0 false PASS trên blocker regression suite và 0 duplicate reservation trong race tests. SC-006: 100% critical fields do accountant xác nhận trước accept. SC-007: không phát hành production nếu chưa finance/security sign-off.

## Open dependencies / assumptions
DO/ERP API và quyền truy cập chưa có; quy định, biểu mẫu, threshold, nguồn vendor/công nợ, authority, retention và data residency chưa được cung cấp. MVP dùng synthetic fixture và manual handoff rõ nhãn. Pilot chỉ với policy và data được duyệt. Không tuyên bố production-ready nếu thiếu các điều kiện này.

## V2 normative supplements
Architecture documents and module requirements E01-R01 through E24-R04 are binding alongside FR-001–028. In conflict, constitution > explicit v2 financial/state/security contracts > module spec > v1-derived general wording. Resolve contradictions through ADR before code. MVP includes a usable three-dossier vertical slice and basic versioned designer; advanced simulation, complex FX, SaaS readiness and live integration are separately gated. No fabricated default company policy.
