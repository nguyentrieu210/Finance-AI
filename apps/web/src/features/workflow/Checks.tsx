import { useEffect, useState } from 'react';
import type { components } from '../../generated/api';

type Check = components['schemas']['CheckResult'];
type Page = { items: Check[]; next_cursor?: string };

export function Checks({ dossierId, token }: { dossierId: string; token: string }) {
  const [items, setItems] = useState<Check[]>([]);
  const [cursor, setCursor] = useState<string>();
  const [requested, setRequested] = useState<string>();
  const [generation, setGeneration] = useState(0);
  const [message, setMessage] = useState('Chưa tải kết quả kiểm tra.');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!generation) return;
    const abort = new AbortController();
    setBusy(true); setMessage('Đang đọc kết quả kiểm tra...');
    const params = new URLSearchParams({ limit: '20' });
    if (requested) params.set('cursor', requested);
    void (async () => {
      try {
        const response = await fetch('/v1/dossiers/' + encodeURIComponent(dossierId) + '/checks?' + params, {
          headers: { Authorization: 'Bearer ' + token }, cache: 'no-store', credentials: 'omit', signal: abort.signal,
        });
        if (abort.signal.aborted) return;
        if (!response.ok) throw new Error(String(response.status));
        const page = await response.json() as Page;
        if (abort.signal.aborted) return;
        if (!Array.isArray(page.items)) throw new Error('invalid response');
        setItems(page.items); setCursor(page.next_cursor);
        setMessage(page.items.length ? 'Kết quả chẩn đoán từ PostgreSQL.' : 'Chưa có kết quả kiểm tra cho phiên bản hiện tại.');
      } catch {
        if (!abort.signal.aborted) { setItems([]); setCursor(undefined); setMessage('Không tải được kết quả kiểm tra.'); }
      } finally {
        if (!abort.signal.aborted) setBusy(false);
      }
    })();
    return () => abort.abort();
  }, [dossierId, token, generation, requested]);

  function load(next?: string) {
    setRequested(next); setGeneration(n => n + 1);
  }

  return <section aria-label="Kết quả kiểm tra hồ sơ">
    <h4>Kết quả kiểm tra hồ sơ</h4>
    <p>Chỉ hiển thị chẩn đoán sandbox đã lưu, không phải phê duyệt tài chính.</p>
    <div className="actions"><button disabled={busy} onClick={() => load()}>Tải kết quả kiểm tra</button>
      <button disabled={busy || !cursor} onClick={() => load(cursor)}>Trang kiểm tra tiếp</button></div>
    <p role="status">{message}</p>
    {items.length > 0 && <div className="table-wrap"><table>
      <thead><tr><th>Quy tắc</th><th>Kết quả</th><th>Mức độ</th><th>Mã lý do</th></tr></thead>
      <tbody>{items.map(row => <tr key={row.id}><td>{row.rule_id}</td><td>{row.outcome}</td><td>{row.severity}</td><td>{row.reason_code}</td></tr>)}</tbody>
    </table></div>}
  </section>;
}
