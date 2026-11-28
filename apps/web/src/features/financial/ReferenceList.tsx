import { useState } from 'react';
import { api } from './financeClient';
import type { components } from '../../generated/api';
type Reference = components['schemas']['Reference'];
type Page = { items: Reference[]; next_cursor?: string };

export function ReferenceList({ title, endpoint, token }: { title: string; endpoint: string; token: string }) {
  const [items, setItems] = useState<Reference[]>([]);
  const [cursor, setCursor] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('Chưa tải.');
  async function load(next?: string) {
    if (busy) return;
    setBusy(true); setMessage('Đang tải dữ liệu đã xác minh...');
    try {
      const params = new URLSearchParams({ limit: '20' }); if (next) params.set('cursor', next);
      const {data: page} = await api<Page>(token, endpoint + '?' + params);
      if (!Array.isArray(page.items)) throw new Error('invalid response');
      setItems(page.items); setCursor(page.next_cursor);
      setMessage('Đã tải ' + page.items.length + ' tham chiếu APPROVED/COMPLETE còn hiệu lực.');
    } catch {
      setItems([]); setCursor(undefined); setMessage('Không tải được dữ liệu tham chiếu.');
    } finally { setBusy(false); }
  }
  return <section aria-label={title}><h4>{title}</h4><div className="actions">
    <button disabled={busy} onClick={() => void load()}>Tải</button>
    <button disabled={busy || !cursor} onClick={() => void load(cursor)}>Trang tiếp</button>
  </div><p role="status">{message}</p>{items.length > 0 && <div className="table-wrap"><table>
    <thead><tr><th>Nguồn</th><th>Trạng thái</th><th>Coverage</th><th>As of</th><th>Bằng chứng nguồn</th></tr></thead>
    <tbody>{items.map(row => <tr key={row.id}><td>{row.source_key}</td><td>{row.state}</td><td>{row.coverage}</td><td>{row.as_of}</td><td>{row.evidence_ids.length ? <details><summary>{row.evidence_ids.length} bằng chứng</summary><ul>{row.evidence_ids.map(id=><li key={id}><code>{id}</code></li>)}</ul></details> : 'Chưa có bằng chứng nguồn'}</td></tr>)}</tbody>
  </table></div>}</section>;
}
