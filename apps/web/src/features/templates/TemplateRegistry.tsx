import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';
type Template = components['schemas']['Template'];

export function TemplateRegistry() {
  const [token, setToken] = useState('');
  const [connected, setConnected] = useState(false);
  const [policy, setPolicy] = useState('00000000-0000-0000-0000-000000001900');
  const [document, setDocument] = useState('00000000-0000-0000-0000-000000001a02');
  const [name, setName] = useState('Synthetic payment template');
  const [items, setItems] = useState<Template[]>([]);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('Chưa kết nối PolicyAdmin.');
  const credential = useRef('');
  const abort = useRef<AbortController | null>(null);
  useEffect(() => () => { abort.current?.abort(); credential.current = ''; }, []);

  async function api(path: string, options: RequestInit = {}) {
    const controller = new AbortController();
    abort.current = controller;
    return fetch(path, {
      ...options,
      headers: { ...options.headers, Authorization: 'Bearer ' + credential.current },
      cache: 'no-store',
      credentials: 'omit',
      signal: controller.signal,
    });
  }

  async function list() {
    if (!credential.current || busy) return;
    setBusy(true);
    try {
      const response = await api('/v1/templates?limit=20');
      if (!response.ok) throw new Error(String(response.status));
      const page = await response.json() as { items: Template[] };
      setItems(page.items);
      setConnected(true);
      setStatus('Đã tải template version hiện tại trong scope.');
    } catch {
      setStatus('Không tải được templates.');
    } finally {
      setBusy(false);
    }
  }

  async function create() {
    if (busy) return;
    setBusy(true);
    try {
      const response = await api('/v1/templates', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'Idempotency-Key': crypto.randomUUID() },
        body: JSON.stringify({ policy_id: policy, name, document_id: document, format: 'PDF' }),
      });
      if (!response.ok) throw new Error(String(response.status));
      setStatus('Đã lưu DRAFT từ CLEAN blob; publish đi cùng policy checker.');
      setBusy(false);
      await list();
    } catch {
      setStatus('Không tạo được template.');
      setBusy(false);
    }
  }

  function clear() {
    abort.current?.abort();
    credential.current = '';
    setToken('');
    setConnected(false);
    setItems([]);
    setBusy(false);
    setStatus('Đã ngắt.');
  }

  return <section aria-labelledby="template-title">
    <h2 id="template-title">Template Registry</h2>
    <p>Digest lấy từ CLEAN blob trong DB; macro-enabled Office source bị chặn ở server.</p>
    <label htmlFor="template-token">Token PolicyAdmin sandbox</label>
    <div className="actions">
      <input id="template-token" type="password" autoComplete="off" maxLength={16384} value={token} disabled={busy || connected} onChange={event => setToken(event.target.value)} />
      <button className="primary" disabled={busy || connected || !token.trim()} onClick={() => { credential.current = token.trim(); setToken(''); void list(); }}>Kết nối</button>
      <button disabled={busy} onClick={clear}>Ngắt</button>
    </div>
    <div className="draft-controls">
      <div><label htmlFor="template-policy">Policy UUID</label><input id="template-policy" value={policy} disabled={!connected || busy} onChange={event => setPolicy(event.target.value)} /></div>
      <div><label htmlFor="template-document">CLEAN document UUID</label><input id="template-document" value={document} disabled={!connected || busy} onChange={event => setDocument(event.target.value)} /></div>
      <div><label htmlFor="template-name">Tên template</label><input id="template-name" value={name} disabled={!connected || busy} onChange={event => setName(event.target.value)} /></div>
      <button className="primary" disabled={!connected || busy} onClick={() => void create()}>Lưu draft</button>
    </div>
    <p role="status">{status}</p>
    <div className="table-wrap"><table>
      <thead><tr><th>Tên</th><th>Format</th><th>Version</th><th>State</th><th>SHA-256</th></tr></thead>
      <tbody>{items.map(row => <tr key={row.id}><td>{row.name}</td><td>{row.format}</td><td>{row.version}</td><td>{row.state}</td><td><code>{row.sha256.slice(0, 16)}…</code></td></tr>)}</tbody>
    </table></div>
  </section>;
}
