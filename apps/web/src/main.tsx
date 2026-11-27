import { StrictMode, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import type { components } from './generated/api';
import './style.css';
import { Drafts } from './features/workflow/Drafts';
import { AuditLog } from './features/audit/AuditLog';
import { Dashboard } from './features/analytics/Dashboard';
import { PublishedPolicies } from './features/policy/PublishedPolicies';

type Organization = components['schemas']['Organization'];
type Page = { items: Organization[]; next_cursor?: string };
const groups: readonly { name: string; items: readonly (readonly [string, string])[] }[] = [
  { name: 'Nền tảng', items: [['E01','Core'],['E02','Danh tính'],['E03','Phân quyền'],['E04','Tổ chức'],['E22','Nhật ký']] },
  { name: 'Chứng từ và AI', items: [['E05','Tài liệu'],['E06','OCR'],['E07','Nhận diện tài liệu'],['E08','Trích xuất'],['E09','Kiểm tra'],['E10','Đối chiếu']] },
  { name: 'Nghiệp vụ tài chính', items: [['E11','Hạn mức'],['E12','Hợp đồng'],['E13','Tạm ứng'],['E14','Ngân sách'],['E15','Quy tắc'],['E16','Chính sách'],['E17','Tri thức']] },
  { name: 'Vận hành', items: [['E18','Quy trình'],['E19','Biểu mẫu'],['E20','Xuất hồ sơ'],['E21','Rủi ro'],['E23','Tích hợp'],['E24','Báo cáo']] },
] as const;

// Deliberately limited to the synthetic test harness. No production login,
// token persistence, refresh flow or real financial data entry.
const sandbox = import.meta.env.VITE_FINANCE_SYNTHETIC === '1';

function App() {
  const [selected, select] = useState('E04');
  const [token, setToken] = useState('');
  const [items, setItems] = useState<Organization[]>([]);
  const [cursor, setCursor] = useState<string | undefined>();
  const [status, setStatus] = useState('Chưa kết nối.');
  const [busy, setBusy] = useState(false);
  const [connected, setConnected] = useState(false);
  const pending = useRef<AbortController | null>(null);
  const generation = useRef(0);
  const credential = useRef('');
  useEffect(() => () => { pending.current?.abort(); credential.current = ''; }, []);
  function clear() {
    generation.current++;
    pending.current?.abort();
    credential.current = '';
    setToken('');
    setItems([]);
    setCursor(undefined);
    setBusy(false);
    setConnected(false);
    setStatus('Đã ngắt kết nối.');
  }
  async function load(next?: string) {
    if (!sandbox || !credential.current) return;
    pending.current?.abort();
    const request = new AbortController();
    pending.current = request;
    const version = ++generation.current;
    setBusy(true);
    setStatus('Đang tải dữ liệu từ API...');
    try {
      const query = new URLSearchParams({ limit: '10' });
      if (next) query.set('cursor', next);
      const response = await fetch('/v1/organizations?' + query, {
        headers: { Authorization: 'Bearer ' + credential.current },
        cache: 'no-store', credentials: 'omit', signal: request.signal,
      });
      if (version !== generation.current) return;
      if (!response.ok) {
        if (response.status === 401 || response.status === 403) {
          credential.current = '';
          setConnected(false);
        }
        setItems([]);
        setCursor(undefined);
        setStatus('Không thể tải dữ liệu (' + response.status + ').');
        return;
      }
      const page = await response.json() as Page;
      if (version !== generation.current) return;
      if (!Array.isArray(page.items)) throw new Error('Invalid response');
      setItems(page.items);
      setCursor(page.next_cursor);
      setConnected(true);
      setStatus('Đã tải ' + page.items.length + ' đơn vị từ API.');
    } catch {
      if (version === generation.current && !request.signal.aborted) {
        setItems([]);
        setCursor(undefined);
        setStatus('Không kết nối được API.');
      }
    } finally {
      if (version === generation.current) setBusy(false);
    }
  }
  const name = groups.flatMap(g => [...g.items]).find(i => i[0] === selected)?.[1] ?? '';
  return <div className="shell">
    <aside aria-label="Điều hướng phân hệ">
      <div className="brand">FINANCE<span>AI</span><small>CONTROL PLATFORM</small></div>
      {groups.map(group => <div className="nav-group" key={group.name}><h2>{group.name}</h2>
        {group.items.map(([id,label]) => <button key={id} aria-current={selected === id ? 'page' : undefined} onClick={() => { if (selected !== id) clear(); select(id); }}><span>{id}</span>{label}</button>)}
      </div>)}
    </aside>
    <div className="workspace">
      <header><span>Không gian quản trị</span><span className="environment">TECHNICAL PREVIEW</span></header>
      <main>
        <p className="eyebrow">FINANCE AI / {selected}</p>
        <h1>{name}</h1>
        <p className="subtitle">Nền tảng kiểm soát hồ sơ tài chính. Chưa được phê duyệt vận hành thực tế.</p>
        {selected === 'E04' && sandbox ? <section aria-labelledby="org-title">
          <h2 id="org-title">Danh sách đơn vị tổ chức</h2>
          <p>Chỉ dùng token thử nghiệm được cấp bởi bộ kiểm thử nội bộ. Không nhập thông tin đăng nhập hoặc token thực tế.</p>
          <div className="sandbox-connection">
            <label htmlFor="sandbox-token">Token sandbox</label>
            <div className="actions"><input id="sandbox-token" type="password" autoComplete="off" value={token} onChange={event => setToken(event.target.value)} disabled={busy || connected} maxLength={16384} />
            <button className="primary" type="button" disabled={busy || connected || !token.trim()} onClick={() => { credential.current = token.trim(); setToken(''); void load(); }}>Kết nối</button><button type="button" onClick={clear}>Ngắt kết nối</button></div>
          </div>
          <p role="status" aria-live="polite">{status}</p>
          <div className="table-wrap"><table><thead><tr><th>Mã đơn vị</th><th>Tên đơn vị</th><th>Loại</th><th>Trạng thái</th></tr></thead>
          <tbody>{items.map(row => <tr key={row.id}><td>{row.code}</td><td>{row.name}</td><td>{row.kind}</td><td>{row.active ? 'Hoạt động' : 'Ngừng hoạt động'}</td></tr>)}</tbody></table></div>
          <div className="actions"><button disabled={!connected || busy} onClick={() => void load()}>Trang đầu</button><button disabled={!cursor || busy} onClick={() => void load(cursor)}>Trang tiếp</button></div>
        </section> : selected === 'E18' && sandbox ? <Drafts /> : selected === 'E22' && sandbox ? <AuditLog /> : selected === 'E24' && sandbox ? <Dashboard /> : selected === 'E16' && sandbox ? <PublishedPolicies /> : <section aria-labelledby="gate-title"><h2 id="gate-title">Chưa mở nghiệp vụ</h2><p>Phân hệ {selected} chưa vượt qua toàn bộ điều kiện nghiệm thu và phân quyền. Không hiển thị số liệu giả, không cho phép gửi hồ sơ hoặc thay đổi dữ liệu.</p><p role="status">Chưa tiếp nhận chứng từ thực tế — chế độ kỹ thuật.</p></section>}
      </main>
    </div>
  </div>;
}
const root = document.getElementById('root');
if (!root) throw new Error('Missing app root');
createRoot(root).render(<StrictMode><App /></StrictMode>);
