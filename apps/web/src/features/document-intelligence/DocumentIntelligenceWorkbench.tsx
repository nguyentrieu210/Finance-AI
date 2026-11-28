import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';

type Document = components['schemas']['Document'];
type Job = components['schemas']['Job'];
type Page = { items: Document[]; next_cursor?: string };

export function DocumentIntelligenceWorkbench() {
  const [token, setToken] = useState('');
  const [connected, setConnected] = useState(false);
  const [dossierId, setDossierId] = useState('');
  const [version, setVersion] = useState('');
  const [docs, setDocs] = useState<Document[]>([]);
  const [kind, setKind] = useState('INVOICE');
  const [reason, setReason] = useState('Phân loại và trích xuất cấu trúc');
  const [job, setJob] = useState<Job | null>(null);
  const [status, setStatus] = useState('Chưa kết nối.');
  const [busy, setBusy] = useState(false);
  const credential = useRef('');

  useEffect(() => () => {
    credential.current = '';
  }, []);

  function disconnect() {
    credential.current = '';
    setToken('');
    setConnected(false);
    setDocs([]);
    setJob(null);
    setStatus('Đã ngắt kết nối.');
  }

  async function loadDocs() {
    const id = dossierId.trim();
    if (!credential.current || !id) return;
    setBusy(true);
    try {
      const dRes = await fetch('/v1/dossiers/' + encodeURIComponent(id), {
        headers: { Authorization: 'Bearer ' + credential.current },
        cache: 'no-store',
        credentials: 'omit',
      });
      const ver = dRes.headers.get('finance-version');
      if (ver) setVersion(ver);
      const listRes = await fetch(`/v1/dossiers/${encodeURIComponent(id)}/documents?limit=20`, {
        headers: { Authorization: 'Bearer ' + credential.current },
        cache: 'no-store',
        credentials: 'omit',
      });
      if (!listRes.ok) {
        setStatus(`Không đọc được chứng từ (${listRes.status}).`);
        return;
      }
      const page = await listRes.json() as Page;
      setDocs(page.items);
      setConnected(true);
      setStatus(`Đã tải ${page.items.length} chứng từ.`);
    } catch {
      setStatus('Lỗi mạng khi tải chứng từ.');
    } finally {
      setBusy(false);
    }
  }

  async function triggerDocAction(docId: string, action: 'classify' | 'extract') {
    const id = dossierId.trim();
    if (!credential.current || !id || !version.trim()) return;
    const payload = action === 'classify' ? { kind: kind.trim() } : { reason: reason.trim() };
    setBusy(true);
    try {
      const res = await fetch(`/v1/dossiers/${encodeURIComponent(id)}/documents/${encodeURIComponent(docId)}/${action}`, {
        method: 'POST',
        headers: {
          Authorization: 'Bearer ' + credential.current,
          'Content-Type': 'application/json',
          'Idempotency-Key': `ui-${action}-` + crypto.randomUUID(),
          'If-Finance-Version': version.trim(),
        },
        body: JSON.stringify(payload),
        cache: 'no-store',
        credentials: 'omit',
      });
      const nextVer = res.headers.get('finance-version');
      if (nextVer) setVersion(nextVer);
      if (!res.ok) {
        setStatus(`${action} thất bại (${res.status}).`);
        return;
      }
      const data = await res.json() as Job;
      setJob(data);
      setStatus(`Đã xếp hàng ${action} job ${data.id}.`);
      await loadDocs();
    } catch {
      setStatus(`Lỗi mạng khi chạy ${action}.`);
    } finally {
      setBusy(false);
    }
  }

  return <section aria-labelledby="e07-title">
    <h2 id="e07-title">Nhận diện &amp; Phân loại Chứng từ (E07)</h2>
    <div className="sandbox-connection">
      <label htmlFor="e07-token">Token Requester sandbox</label>
      <div className="actions">
        <input id="e07-token" type="password" autoComplete="off" value={token} onChange={e => setToken(e.target.value)} disabled={busy || connected} maxLength={16384} />
        <button className="primary" type="button" disabled={busy || connected || !token.trim()} onClick={() => { credential.current = token.trim(); setToken(''); setConnected(true); setStatus('Đã nạp token.'); void loadDocs(); }}>Kết nối</button>
        <button type="button" onClick={disconnect}>Ngắt kết nối</button>
      </div>
    </div>
    <div className="actions">
      <input value={dossierId} onChange={e => setDossierId(e.target.value)} placeholder="Dossier UUID" aria-label="Dossier UUID" />
      <input value={version} onChange={e => setVersion(e.target.value)} placeholder="If-Finance-Version" aria-label="If-Finance-Version" />
      <input value={kind} onChange={e => setKind(e.target.value)} placeholder="INVOICE" aria-label="Loại chứng từ" />
      <input value={reason} onChange={e => setReason(e.target.value)} placeholder="Lý do extract" aria-label="Lý do extract" />
      <button type="button" disabled={!connected || busy || !dossierId.trim()} onClick={() => void loadDocs()}>Tải danh sách</button>
    </div>
    <p role="status" aria-live="polite">{status}</p>
    <div className="table-wrap"><table><thead><tr><th>Document ID</th><th>Tên file</th><th>Loại hiện tại</th><th>Thao tác</th></tr></thead>
      <tbody>{docs.map(doc => <tr key={doc.id}>
        <td>{doc.id}</td><td>{doc.file_name}</td><td>{doc.kind}</td>
        <td className="actions">
          <button type="button" disabled={busy || !version.trim()} onClick={() => void triggerDocAction(doc.id, 'classify')}>Classify</button>
          <button type="button" disabled={busy || !version.trim()} onClick={() => void triggerDocAction(doc.id, 'extract')}>Extract</button>
        </td>
      </tr>)}</tbody>
    </table></div>
    {job && <p>Job {job.id}: {job.state}</p>}
  </section>;
}
