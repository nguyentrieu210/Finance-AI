import { useEffect, useRef, useState } from 'react';
import type { components, operations } from '../../generated/api';

type Item = components['schemas']['ChecklistItem'];
type Page = operations['getChecklist']['responses'][200]['content']['application/json'];

const labels: Record<Item['state'], string> = {
  PRESENT: 'Đã có chứng từ',
  MISSING: 'Thiếu chứng từ',
  INVALID: 'Không hợp lệ',
  UNKNOWN: 'Chưa xác minh',
  NOT_APPLICABLE: 'Không áp dụng',
};

export function Checklist({ dossierId, token }: { dossierId: string; token: string }) {
  const [items, setItems] = useState<Item[]>([]);
  const [cursor, setCursor] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('Chưa tải danh mục chứng từ.');
  const active = useRef<AbortController | null>(null);

  useEffect(() => () => {
    active.current?.abort();
    active.current = null;
  }, [dossierId, token]);

  async function load(next?: string) {
    active.current?.abort();
    const controller = new AbortController();
    active.current = controller;
    setBusy(true);
    setItems([]);
    setCursor(undefined);
    setMessage('Đang tải checklist...');
    const params = new URLSearchParams({ limit: '20' });
    if (next) params.set('cursor', next);
    try {
      const response = await fetch('/v1/dossiers/' + encodeURIComponent(dossierId) + '/checklist?' + params, {
        headers: { Authorization: 'Bearer ' + token },
        credentials: 'omit', cache: 'no-store', signal: controller.signal,
      });
      if (!response.ok) throw new Error(String(response.status));
      const page = await response.json() as Page;
      if (!Array.isArray(page.items)) throw new Error('invalid response');
      if (controller.signal.aborted) return;
      setItems(page.items);
      setCursor(page.next_cursor);
      setMessage(page.items.length
        ? 'Checklist từ phiên kiểm tra hiện tại. Không đồng nghĩa hồ sơ đã đủ điều kiện phê duyệt.'
        : next ? 'Không còn mục trên trang tiếp theo.' : 'Chưa có checklist cho phiên kiểm tra hiện tại; không kết luận hồ sơ đã đầy đủ.');
    } catch {
      if (!controller.signal.aborted) setMessage('Không tải được checklist hoặc không có quyền truy cập.');
    } finally {
      if (!controller.signal.aborted) setBusy(false);
    }
  }

  return <section aria-label="Danh mục chứng từ cần hoàn thiện">
    <h4>Danh mục chứng từ cần hoàn thiện</h4>
    <div className="actions">
      <button disabled={busy || !token} onClick={() => void load()}>Tải checklist</button>
      <button disabled={busy || !cursor} onClick={() => void load(cursor)}>Trang checklist tiếp</button>
    </div>
    <p role="status">{message}</p>
    {items.length > 0 && <div className="table-wrap"><table>
      <thead><tr><th>Yêu cầu</th><th>Trạng thái</th><th>Điều khoản</th><th>Bằng chứng</th></tr></thead>
      <tbody>{items.map(row => <tr key={row.id}>
        <td>{row.requirement}</td><td>{labels[row.state] ?? row.state}</td><td>{row.clause_id}</td>
        <td>{row.evidence_ids.length ? row.evidence_ids.join(', ') : 'Chưa có liên kết'}</td>
      </tr>)}</tbody>
    </table></div>}
  </section>;
}
