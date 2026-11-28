# UI/UX — screen contracts
Responsive Vietnamese SPA. Route guards convenience only; server validates. Common loading/empty/error/forbidden/stale/retry states; keyboard, accessible labels, no color-only status. Currency and timezone explicit. Never cache financial payload in offline PWA. All mutations disabled on stale revision until refresh; show conflict details. Three-pane reviewer: document list | PDF viewer highlighting locator | rule results and evidence. Primary actions use reason/confirm dialogs when needed; no auto approve.

| Screen | Route | Main interaction / acceptance |
|---|---|---|
| Dashboard | / | scoped KPI, queue age; no hidden cross-company aggregate |
| My dossiers | /dossiers | filter status/kind, pagination, create |
| Create | /dossiers/new | choose company/kind, policy availability, form |
| Detail | /dossiers/:id | timeline, revision, next allowed action |
| Upload | /dossiers/:id/documents | progress/quarantine/retry, no unsafe preview |
| Document viewer | /dossiers/:id/viewer | page/cell highlight and original hash |
| Field review | /dossiers/:id/fields | candidate vs source, correction reason |
| Matching | /dossiers/:id/matching | paired lines, ambiguous match resolution |
| Checklist | /dossiers/:id/checklist | required/missing/unknown, policy citation |
| Preflight | /dossiers/:id/checks | outcome + severity + evidence, rerun |
| Waivers | /dossiers/:id/waivers | request/decision, independent approver |
| Export | /dossiers/:id/exports | render job, versioned download manifest |
| Handoff | /dossiers/:id/handoffs | manual receipt vs API ack, unknown status |
| Review queue | /review | assignment and priorities |
| Accountant workspace | /review/:id | critical confirmations, return/accept |
| Payments | /dossiers/:id/payment | source read-only, status and evidence |
| Policy list | /admin/policies | effective versions, draft/create |
| Policy editor | /admin/policies/:id | clauses, diff, publish request |
| Rule designer | /admin/rules/:id | typed operands, validation, draft |
| Simulator | /admin/simulation | corpus selection, compare outcomes |
| Form designer | /admin/forms/:id | fields/conditions, preview |
| Templates | /admin/templates | upload, mapping, render sample |
| Workflow | /admin/workflows | transition table/guards, validation |
| Reference imports | /admin/references | mapping, coverage, independent verification |
| Reservations | /finance/reservations | available/held/consumed, reconciliation |
| Organization | /admin/organization | membership, role scopes, effective dates |
| Risk queue | /risk | signal resolution and evidence |
| Reconciliation | /integration/reconciliation | unknown external attempts, source evidence |
| Audit | /audit | filters, event chain, scoped export |
| Operations | /ops | queue health, failures, redacted cost metrics |

Detailed wireframes deferred; route and acceptance are implementation contract. E18 owns dossier/review screens, each other module owns its feature UI, shared shell owned foundation.


## Quy tắc Frontend bắt buộc — yêu cầu chủ dự án
- Không dùng khối giới thiệu/mô tả đầu trang (page intro/header/eyebrow): tiêu đề lớn, dòng mô tả chung, khoảng trống và nút CTA chung kiểu nv-page-title. Áp dụng cho tất cả màn hình, không tạo lại dưới tên component khác.
- Nội dung nghiệp vụ bắt đầu ngay dưới thanh điều hướng. Tiêu đề, bộ lọc, thao tác chỉ đặt tại toolbar hoặc khối chức năng thực sự cần; không lặp lại tên menu hay mô tả trang. Ưu tiên mật độ thông tin và không gian thao tác.

- Dùng Tailwind CSS và thang spacing 4px; mặc định Compact ERP: nội dung full width, padding 8–12px, gap 4–8px, toolbar/input 32px, hàng bảng khoảng 30px; không bọc nhiều lớp card hoặc để khoảng trống trang trí. Mobile giữ vùng thao tác tối thiểu 40px.

## Cổng xác thực bắt buộc
- Mọi trang nghiệp vụ và tài nguyên frontend phải qua kiểm tra phiên ở máy chủ trước khi trả nội dung. Không có guest/demo/preview bypass bằng URL, localStorage, sessionStorage hay cờ frontend. Chỉ endpoint xác thực, tài nguyên giao diện đăng nhập và health được công khai. API tài chính chỉ mở khi có xác thực và phân quyền nghiệp vụ thực tế.
