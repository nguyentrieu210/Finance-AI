import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';

type Document = components['schemas']['Document'];
type UploadGrant = components['schemas']['UploadGrant'];
type Job = components['schemas']['Job'];
type Download = components['schemas']['Download'];
type Page = { items: Document[]; next_cursor?: string };

export function DocumentsWorkbench() {
  const [token, setToken] = useState('');
  const [connected, setConnected] = useState(false);
  const [dossierId, setDossierId] = useState('');
  const [version, setVersion] = useState('');
  const [docs, setDocs] = useState<Document[]>([]);
  const [grant, setGrant] = useState<UploadGrant | null>(null);
  const [downloadInfo, setDownloadInfo] = useState<Download | null>(null);
  const [job, setJob] = useState<Job | null>(null);
  const [replacementId, setReplacementId] = useState('');
  const [reason, setReason] = useState('Thay thế bản chứng từ hợp lệ');
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
    setGrant(null);
    setDownloadInfo(null);
    setJob(null);
    setStatus('Đã ngắt kết nối.');
  }

  async function loadDossierDocs() {
    const id = dossierId.trim();
    if (!credential.current || !id) return;
    setBusy(true);
    setStatus('Đang tải hồ sơ và danh sách chứng từ...');
    try {
      const dRes = await fetch('/v1/dossiers/' + encodeURIComponent(id), {
        headers: { Authorization: 'Bearer ' + credential.current },
        cache: 'no-store',
        credentials: 'omit',
      });
      if (!dRes.ok) {
        if (dRes.status === 401 || dRes.status === 403) {
          credential.current = '';
          setConnected(false);
        }
        setStatus(`Không đọc được hồ sơ (${dRes.status}).`);
        return;
      }
      const ver = dRes.headers.get('finance-version');
      if (ver) setVersion(ver);
      const listRes = await fetch('/v1/dossiers/' + encodeURIComponent(id) + '/documents?limit=20', {
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

  async function issueGrant(docId: string) {
    const id = dossierId.trim();
    if (!credential.current || !id || !version.trim()) return;
    setBusy(true);
    setStatus(`Đang cấp upload grant cho ${docId}...`);
    try {
      const res = await fetch(`/v1/dossiers/${encodeURIComponent(id)}/documents/${encodeURIComponent(docId)}/upload-grants`, {
        method: 'POST',
        headers: {
          Authorization: 'Bearer ' + credential.current,
          'Idempotency-Key': 'ui-grant-' + crypto.randomUUID(),
          'If-Finance-Version': version.trim(),
        },
        cache: 'no-store',
        credentials: 'omit',
      });
      const nextVer = res.headers.get('finance-version');
      if (nextVer) setVersion(nextVer);
      if (!res.ok) {
        setStatus(`Cấp grant thất bại (${res.status}).`);
        return;
      }
      const data = await res.json() as UploadGrant;
      setGrant(data);
      setStatus(`Đã cấp grant generation=${data.generation}.`);
    } catch {
      setStatus('Lỗi mạng khi cấp upload grant.');
    } finally {
      setBusy(false);
    }
  }

  async function finalizeDoc(doc: Document) {
    const id = dossierId.trim();
    const gen = grant && grant.document_id === doc.id ? grant.generation : 1;
    if (!credential.current || !id || !version.trim()) return;
    setBusy(true);
    setStatus(`Đang finalize ${doc.id} (gen ${gen})...`);
    try {
      const res = await fetch(`/v1/dossiers/${encodeURIComponent(id)}/documents/${encodeURIComponent(doc.id)}/finalize`, {
        method: 'POST',
        headers: {
          Authorization: 'Bearer ' + credential.current,
          'Content-Type': 'application/json',
          'Idempotency-Key': 'ui-fin-' + crypto.randomUUID(),
          'If-Finance-Version': version.trim(),
        },
        body: JSON.stringify({ sha256: doc.sha256, generation: gen }),
        cache: 'no-store',
        credentials: 'omit',
      });
      const nextVer = res.headers.get('finance-version');
      if (nextVer) setVersion(nextVer);
      if (!res.ok) {
        setStatus(`Finalize thất bại (${res.status}).`);
        return;
      }
      const data = await res.json() as Job;
      setJob(data);
      setStatus(`Đã gửi finalize job ${data.id}.`);
      await loadDossierDocs();
    } catch {
      setStatus('Lỗi mạng khi finalize.');
    } finally {
      setBusy(false);
    }
  }

  async function supersedeDoc(docId: string) {
    const id = dossierId.trim();
    const rep = replacementId.trim();
    if (!credential.current || !id || !rep || !version.trim() || !reason.trim()) return;
    setBusy(true);
    setStatus(`Đang thay thế ${docId} bằng ${rep}...`);
    try {
      const res = await fetch(`/v1/dossiers/${encodeURIComponent(id)}/documents/${encodeURIComponent(docId)}/supersede`, {
        method: 'POST',
        headers: {
          Authorization: 'Bearer ' + credential.current,
          'Content-Type': 'application/json',
          'Idempotency-Key': 'ui-sup-' + crypto.randomUUID(),
          'If-Finance-Version': version.trim(),
        },
        body: JSON.stringify({ replacement_id: rep, reason: reason.trim() }),
        cache: 'no-store',
        credentials: 'omit',
      });
      const nextVer = res.headers.get('finance-version');
      if (nextVer) setVersion(nextVer);
      if (!res.ok) {
        setStatus(`Thay thế thất bại (${res.status}).`);
        return;
      }
      setStatus(`Đã supersede chứng từ ${docId}.`);
      await loadDossierDocs();
    } catch {
      setStatus('Lỗi mạng khi supersede.');
    } finally {
      setBusy(false);
    }
  }

  async function requestDownload(docId: string) {
    const id = dossierId.trim();
    if (!credential.current || !id) return;
    setBusy(true);
    try {
      const res = await fetch(`/v1/dossiers/${encodeURIComponent(id)}/documents/${encodeURIComponent(docId)}/download`, {
        headers: { Authorization: 'Bearer ' + credential.current },
        cache: 'no-store',
        credentials: 'omit',
      });
      if (!res.ok) {
        setStatus(`Không lấy được link tải (${res.status}).`);
        return;
      }
      const data = await res.json() as Download;
      setDownloadInfo(data);
      setStatus(`Đã tạo URL tải ngắn hạn (SHA-256 ${data.sha256.slice(0, 12)}...).`);
    } catch {
      setStatus('Lỗi mạng khi yêu cầu download.');
    } finally {
      setBusy(false);
    }
  }

  return <section aria-labelledby="e05-title">
    <h2 id="e05-title">Vòng đời Chứng từ &amp; Upload Grant (E05)</h2>
    <div className="sandbox-connection">
      <label htmlFor="e05-token">Token Requester sandbox</label>
      <div className="actions">
        <input id="e05-token" type="password" autoComplete="off" value={token} onChange={e => setToken(e.target.value)} disabled={busy || connected} maxLength={16384} />
        <button className="primary" type="button" disabled={busy || connected || !token.trim()} onClick={() => { credential.current = token.trim(); setToken(''); setConnected(true); setStatus('Đã nạp token.'); void loadDossierDocs(); }}>Kết nối</button>
        <button type="button" onClick={disconnect}>Ngắt kết nối</button>
      </div>
    </div>
    <div className="actions">
      <input value={dossierId} onChange={e => setDossierId(e.target.value)} placeholder="Dossier UUID" aria-label="Dossier UUID" />
      <input value={version} onChange={e => setVersion(e.target.value)} placeholder="If-Finance-Version" aria-label="If-Finance-Version" />
      <button type="button" disabled={!connected || busy || !dossierId.trim()} onClick={() => void loadDossierDocs()}>Tải chứng từ</button>
    </div>
    <div className="actions">
      <input value={replacementId} onChange={e => setReplacementId(e.target.value)} placeholder="Replacement Document UUID" aria-label="Replacement UUID" />
      <input value={reason} onChange={e => setReason(e.target.value)} placeholder="Lý do supersede" aria-label="Lý do supersede" />
    </div>
    <p role="status" aria-live="polite">{status}</p>
    <div className="table-wrap"><table><thead><tr><th>ID</th><th>Tên file</th><th>Loại</th><th>Trạng thái</th><th>Thao tác</th></tr></thead>
      <tbody>{docs.map(doc => <tr key={doc.id}>
        <td>{doc.id}</td><td>{doc.file_name}</td><td>{doc.kind}</td><td>{doc.state}</td>
        <td className="actions">
          <button type="button" disabled={busy} onClick={() => void issueGrant(doc.id)}>Upload Grant</button>
          <button type="button" disabled={busy} onClick={() => void finalizeDoc(doc)}>Finalize</button>
          <button type="button" disabled={busy || !replacementId.trim()} onClick={() => void supersedeDoc(doc.id)}>Supersede</button>
          <button type="button" disabled={busy} onClick={() => void requestDownload(doc.id)}>Download</button>
        </td>
      </tr>)}</tbody>
    </table></div>
    {grant && <p>Grant gen {grant.generation}: {grant.upload_url} (checksum {grant.checksum_sha256_base64})</p>}
    {job && <p>Job {job.id}: {job.state}</p>}
    {downloadInfo && <p>Download URL: {downloadInfo.url} (hết hạn {downloadInfo.expires_at})</p>}
  </section>;
}
