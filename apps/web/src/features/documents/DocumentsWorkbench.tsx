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
  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [uploaded, setUploaded] = useState<string | null>(null);
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
    setSelectedFile(null);
    setUploaded(null);
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
      setUploaded(null);
      setStatus(`Đã cấp grant generation=${data.generation}.`);
    } catch {
      setStatus('Lỗi mạng khi cấp upload grant.');
    } finally {
      setBusy(false);
    }
  }

  async function uploadSelected() {
    if (!grant || !selectedFile || !credential.current) return;
    const doc = docs.find(item => item.id === grant.document_id);
    if (!doc || selectedFile.size < 1 || selectedFile.size > 25 * 1024 * 1024) return;
    setBusy(true);
    try {
      const target = new URL(grant.upload_url);
      if (target.hostname === 'upload.synthetic.local' || (target.protocol !== 'https:' && target.hostname !== '127.0.0.1')) {
        setStatus('Storage chưa được cấu hình hoặc URL không hợp lệ.');
        return;
      }
      if (Date.parse(grant.expires_at) <= Date.now()) { setStatus('Grant hết hạn.'); return; }
      const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', await selectedFile.arrayBuffer()));
      const hash = Array.from(digest, b => b.toString(16).padStart(2, '0')).join('');
      if (hash !== doc.sha256 || btoa(String.fromCharCode(...digest)) !== grant.checksum_sha256_base64) {
        setStatus('File không khớp SHA-256 chứng từ.'); return;
      }
      const response = await fetch(target, {
        method: 'PUT', headers: {
          'Content-Type': grant.content_type,
          'If-None-Match': '*',
          'x-amz-checksum-sha256': grant.checksum_sha256_base64,
        }, body: selectedFile, credentials: 'omit', redirect: 'error',
        cache: 'no-store', referrerPolicy: 'no-referrer',
      });
      if (!response.ok) { setStatus(`Upload thất bại (${response.status}).`); return; }
      setUploaded(grant.document_id);
      setStatus('Đã tải file; có thể xác minh và đưa vào hàng đợi quét.');
    } catch { setStatus('Không tải được file; kiểm tra kết nối và CORS storage.'); }
    finally { setBusy(false); }
  }

  async function finalizeDoc(doc: Document) {
    const id = dossierId.trim();
    if (!grant || grant.document_id !== doc.id || uploaded !== doc.id) { setStatus('Cần upload đúng file trước khi finalize.'); return; }
    const gen = grant.generation;
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
          <button type="button" disabled={busy || uploaded !== doc.id} onClick={() => void finalizeDoc(doc)}>Finalize</button>
          <button type="button" disabled={busy || !replacementId.trim()} onClick={() => void supersedeDoc(doc.id)}>Supersede</button>
          <button type="button" disabled={busy} onClick={() => void requestDownload(doc.id)}>Download</button>
        </td>
      </tr>)}</tbody>
    </table></div>
    {grant && <div className="actions">
      <span>Grant thế hệ {grant.generation}, hết hạn {grant.expires_at}</span>
      <input type="file" aria-label="Chọn chứng từ" disabled={busy} onChange={e => { setSelectedFile(e.target.files?.[0] ?? null); setUploaded(null); }} />
      <button type="button" disabled={busy || !selectedFile} onClick={() => void uploadSelected()}>Tải file lên</button>
    </div>}
    {job && <p>Job {job.id}: {job.state}</p>}
    {downloadInfo && <p>Download URL: {downloadInfo.url} (hết hạn {downloadInfo.expires_at})</p>}
  </section>;
}
