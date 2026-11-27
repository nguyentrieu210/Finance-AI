import { createRoot } from 'react-dom/client';
import { StrictMode } from 'react';
import './style.css';
function App() {
  return <main><header>FINANCE AI / PLATFORM</header><section aria-labelledby="heading"><p className="eyebrow">G00 · NỀN TẢNG KỸ THUẬT</p><h1 id="heading">Kiểm soát hồ sơ tài chính</h1><p>Hệ thống đang trong giai đoạn xây dựng. Chưa tiếp nhận chứng từ, chưa phê duyệt và chưa thanh toán.</p><p role="status">Các nghiệp vụ được khóa cho đến khi hoàn thành kiểm soát truy cập và kiểm thử tích hợp.</p></section></main>;
}
const root = document.getElementById('root');
if (!root) throw new Error('Missing app root');
createRoot(root).render(<StrictMode><App /></StrictMode>);
