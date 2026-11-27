# Finance AI Constitution
Version 2.0.0 | Draft 2026-11-27 | Ratification pending

## I. Rust-first, deterministic core
Backend, worker, domain, financial math, workflow, matching, rule evaluation, audit và integration MUST viết Rust. OCR/model/office converter MAY là dịch vụ ngoài qua adapter; frontend React TypeScript. Không tái triển khai thư viện OCR chỉ để đạt tỷ lệ Rust. Tiền dùng Decimal + PostgreSQL NUMERIC, không float; currency, rounding, scale explicit. Mọi quy tắc xác định được phải chạy deterministic, không phụ thuộc LLM.

## II. Evidence before trust
Mỗi giá trị nhận diện MUST truy về tài liệu bất biến, hash, trang/cell/bounding box khi có, extractor/version, raw value, confidence và lịch sử xác nhận. AI không tự duyệt hồ sơ, không tự tạo căn cứ. UNKNOWN hoặc ERROR không biến thành PASS; rule bắt buộc có UNKNOWN/ERROR phải chặn hoặc chuyển xác minh theo chính sách được duyệt. Không có dữ liệu lịch sử không đồng nghĩa không trùng.

## III. Human authority and separation of duties
Người lập không tự duyệt hoặc tự chấp thuận ngoại lệ của mình. Kế toán xác nhận thông số trọng yếu; ngoại lệ phải có người có thẩm quyền, lý do, bằng chứng, phạm vi và thời hạn. Rule không cho override phải bị chặn. App không chuyển tiền. DO/ERP là nguồn chính thức cho trạng thái tương ứng khi tích hợp được.

## IV. Secure by default
OIDC, kiểm tra authorization server-side từng resource, tenant isolation, deny by default, least privilege. Chứng từ mã hóa khi lưu/truyền, signed URL ngắn hạn, quarantine và malware scan; giới hạn kích thước/giải nén. Văn bản upload là dữ liệu không đáng tin, không phải instruction cho AI; model không có quyền ghi DB hoặc gọi tool thanh toán. Không log PII, secrets, tài khoản ngân hàng đầy đủ. Provider AI phải được duyệt trước khi gửi dữ liệu thật.

## V. Reproducible and versioned
Chứng từ, policy, biểu mẫu, rule, workflow và kết quả review versioned. Hồ sơ đã trình khóa snapshot; sửa tạo revision mới, vô hiệu approval/waiver liên quan. Audit append-only, transaction atomic, outbox/idempotency, optimistic locking và đối soát. Lưu thời điểm UTC, hiển thị timezone cấu hình. Không xóa hồ sơ đã trình ngoài quy trình retention được phê duyệt.

## VI. Test-first, real evidence
Viết test trước hoặc cùng thay đổi. Kiểm thử domain, PostgreSQL thật, API, E2E, tenant isolation, duplicate race, AI evaluation và recovery. Mock chỉ dùng unit/contract; không dùng mock để tuyên bố integration production hoàn tất. Không bỏ test, sửa kỳ vọng vô căn cứ hay đánh dấu done khi CI đỏ. Fixture không chứa dữ liệu thật chưa được duyệt.

## VII. Simplicity and operability
Modular monolith + Cargo workspace, một PostgreSQL và object storage. Tách worker khi cần; không Kafka/Kubernetes/microservices/Temporal trong MVP nếu chưa có ADR. Schema migration có kế hoạch phục hồi; observability, backup và restore drill bắt buộc trước production.

## Governance
Thay đổi constitution phải có ADR, đánh giá impact và approval của product owner, tech lead, finance owner đối với rule nghiệp vụ, security owner đối với dữ liệu. Mọi feature spec/plan/tasks tham chiếu constitution. Không được bỏ qua nguyên tắc để đạt tiến độ.

## VIII. Contract ownership and release truth
Một active feature 001; E01–E24 là work packages logic, không phải microservice. Contract registry, migration IDs và public Rust traits chỉ thay qua ADR + integration lead. Mandatory unknown/error fail closed; outcome độc lập severity. Tất cả mutation tài chính dùng transaction, idempotency, scope và audit. Không đánh dấu DONE khi chỉ có spec, stub hay CI chưa chạy. Foundation và technical pilot không đồng nghĩa production. Không chuyển tiền, không tạo bút toán chính thức.
