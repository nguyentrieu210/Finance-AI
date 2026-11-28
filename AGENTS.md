# Agent contract
Đọc constitution, active spec, module spec, API/DB contracts, task graph và acceptance trước khi sửa. Chỉ nhận task có dependency DONE bằng evidence. Không thay interface/migration shared khi chưa được integration lead duyệt. Mỗi agent worktree/branch riêng, paths owner rõ, commit nhỏ. Unit mock được phép, real DB/object storage tests bắt buộc tại gate. Không log secret, không đưa tài liệu thật vào fixtures, không gửi AI nếu policy chưa duyệt. Không tự duyệt hồ sơ, không có endpoint chuyển tiền. Không đánh dấu DONE khi test chưa chạy; ghi commit SHA, lệnh, exit code, artifact, test IDs, known gaps. Nếu bị blocker ghi BLOCKED và tiếp task độc lập. Không tự bỏ scope. Merge queue kiểm tra integrated HEAD, không cherry-pick mù.


## Quy tắc Frontend bắt buộc — yêu cầu chủ dự án
- Không dùng khối giới thiệu/mô tả đầu trang (page intro/header/eyebrow): tiêu đề lớn, dòng mô tả chung, khoảng trống và nút CTA chung kiểu nv-page-title. Áp dụng cho tất cả màn hình, không tạo lại dưới tên component khác.
- Nội dung nghiệp vụ bắt đầu ngay dưới thanh điều hướng. Tiêu đề, bộ lọc, thao tác chỉ đặt tại toolbar hoặc khối chức năng thực sự cần; không lặp lại tên menu hay mô tả trang. Ưu tiên mật độ thông tin và không gian thao tác.

- Dùng Tailwind CSS và thang spacing 4px; mặc định Compact ERP: nội dung full width, padding 8–12px, gap 4–8px, toolbar/input 32px, hàng bảng khoảng 30px; không bọc nhiều lớp card hoặc để khoảng trống trang trí. Mobile giữ vùng thao tác tối thiểu 40px.

## Cổng xác thực bắt buộc
- Mọi trang nghiệp vụ và tài nguyên frontend phải qua kiểm tra phiên ở máy chủ trước khi trả nội dung. Không có guest/demo/preview bypass bằng URL, localStorage, sessionStorage hay cờ frontend. Chỉ endpoint xác thực, tài nguyên giao diện đăng nhập và health được công khai. API tài chính chỉ mở khi có xác thực và phân quyền nghiệp vụ thực tế.
