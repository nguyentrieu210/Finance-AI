# Finance AI Platform — Spec Kit v2.0 Enterprise Architecture Edition
27/11/2026 | Draft for review — chưa phê duyệt nghiệp vụ.

Nâng cấp v1.0, giữ quy trình 6 bước và Rust-native core. Một feature active `001-financial-control`, 24 work packages E01–E24. Tất cả task chưa thực hiện. Đây là tài liệu đặc tả, không phải app, CLI hay CI đã chạy.

## Entry points
- `.specify/memory/constitution.md`: nguyên tắc.
- `specs/001-financial-control/spec.md`, `plan.md`, `research.md`, `data-model.md`, `tasks.md`: active Spec Kit.
- `specs/001-financial-control/modules/`: đặc tả theo engine; ID requirement E##-R##.
- `contracts/openapi.yaml`, `rule-dsl.schema.json`: hợp đồng draft; freeze tại G00/G01 sau review.
- `architecture/`: financial semantics, workflow, policy, document pipeline, threat model, UI, release gates.
- `agents/`: ownership, task DAG, execution rules.
- `tests/`: acceptance catalog và traceability; không phải executable tests.
- `scripts/validate_spec.py`: static validation; chạy `python scripts/validate_spec.py` với PyYAML, optional jsonschema.
- `docs/review-closure.md`: mapping F01–F15 và residual risk.

## Khởi động
Khởi tạo Specify CLI chính thức trong repo mới theo https://github.com/github/spec-kit; copy gói vào repo (không ghi đè scripts/templates CLI). Chọn feature 001, đọc AGENTS, constitution, chạy clarify/analyze, freeze contracts rồi implement theo DAG. Không tự phát sinh feature 24 lần. Tích hợp DO/ERP thật bị gated bởi owner/API/sandbox/credentials. Dữ liệu thử nghiệm synthetic hoặc đã được duyệt. V2 supersedes v1; không merge lẫn hai bộ spec.
