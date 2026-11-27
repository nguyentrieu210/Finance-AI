# Financial semantics — normative draft
## Monetary and accounting boundaries
App kiểm soát hồ sơ, không sổ cái chính thức, không chuyển tiền. ERP/nguồn xác nhận độc lập là authoritative cho issued/paid/refund/reversed. Money = decimal string + ISO currency; không binary float. Precision 28, scale 8 tối đa; scale thanh toán, phương pháp làm tròn, tỷ giá và ngày hiệu lực lấy từ policy được duyệt. Không tự chuyển currency hoặc clamp số âm. Quantities có UOM và conversion version. Công thức dưới đây là invariant thiết kế; chính sách thuế, giữ lại và khấu trừ thực tế chưa duyệt.

## Balances and disjoint buckets
Mỗi resource có khóa scope, currency, effective version, coverage. Approved cap C = giá trị gốc + phụ lục có hiệu lực được duyệt, hoặc ngân sách/obligation/invoice/advance cap tương ứng. P = tổng confirmed consumption trừ reversal có bằng chứng. R = tổng phần reservation còn hiệu lực chưa được consumption; không tính lại khoản đã nằm trong P. Available A = C - P - R. Nếu thiếu coverage, stale source hoặc A âm: UNKNOWN hoặc FAIL theo rule; không coi là 0 hay PASS. ERP snapshot phải có watermark và danh sách external IDs (hoặc đối soát tương đương) để loại overlap local effects. Không cộng số dư tổng hợp với cùng giao dịch chi tiết.

Payment payable = min của các cap áp dụng đã xác minh: contractual obligation due, accepted eligible value, invoice remaining, contract available, budget available; tất cả cùng currency hoặc approved FX. Non-applicable cap phải được policy xác định. Requested positive <= payable. Net payable = approved gross eligible - discount (nếu chưa tính trong gross) - retention - advance deduction - withholding (nếu áp dụng); thuế và chiết khấu phải có thứ tự công thức policy, không trừ hai lần. Nếu net âm, chuyển ngoại lệ; không tạo giao dịch âm ngầm. Retention release có obligation riêng. Dữ liệu thuế, điều chỉnh invoice và partial payment phải có liên kết nguồn; chỉ reversal đã xác nhận mới giảm consumption.

Advance remaining = issued confirmed - settled eligible allocated - confirmed refunds; các bucket không chồng lắp. Settlement eligible expenses E, applied to advance S <= remaining (trừ active reservations); excess E-S chỉ tạo hồ sơ chi bổ sung nếu policy cho phép. Refund due = remaining after S khi final settlement; không ghi confirmed refund chỉ từ đề nghị. Partial settlement không ép refund trước thời hạn policy. Multi-advance allocations atomically reserve per advance, cùng currency trừ policy FX đã duyệt. Không tiêu thụ lại cùng invoice expense trên settlement và payment.

## Reservation commands
| Command | Source → target | Guard / effect |
|---|---|---|
| reserve | none → RESERVED | acquire all resource locks sorted by scoped ID; validate current revision, cap and freshness, write all lines or none |
| commit | RESERVED → COMMITTED | handoff attempt created; durable outbox; retain cap during uncertain DO status |
| consume | RESERVED/COMMITTED → PARTIAL/CONSUMED | confirmed unique source event, decrement remaining, append consumption effect in same txn |
| release | RESERVED → RELEASED | authorized cancellation or expiry only if no external dispatch possible |
| release committed | COMMITTED/PARTIAL → RELEASED (remaining only) | verified terminal cancel/reject and reconciliation confirms no further payment |
| reverse | CONSUMED/PARTIAL → reconciliation required | confirmed external reversal; append negative effect referencing original; no automatic reopening of approval |

Reserve retry uses command ID + canonical request hash. Different payload same key -> 409. Partial consumption splits amount into consumed and remaining; immutable effects. Lock acquisition + idempotency + audit/outbox are atomic. Retry serialization conflict bounded. No network calls under lock. Stale source or lost ack triggers review; no TTL release for committed/unknown. Two simultaneous 60 requests against 100 cap: at most one succeeds. A multi-cap failure rolls back all holds. Release/consume race resolves with lock and event id. Tests require real PostgreSQL with independent connections.
