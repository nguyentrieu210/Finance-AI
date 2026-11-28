import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';

type Job = components['schemas']['Job'];
type Evidence = components['schemas']['Evidence'];

export function OcrWorkbench() {
  const [token, setToken] = useState('');
  const [connected, setConnected] = useState(false);
  const [dossierId, setDossierId] = useState('');
  const [documentId, setDocumentId] = useState('');
  const [version, setVersion] = useState('');
  const [reason, setReason] = useState('Chạy lại OCR tổng hợp');
  const [evidenceId, setEvidenceId] = useState('00000000-0000-0000-0000-000000002333');
  const [job, setJob] = useState<Job | null>(null);
  const [evidence, setEvidence] = useState<Evidence | null>(null);
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
    setJob(null);
    setEvidence(null);
    setStatus('Đã ngắt kết nối.');
  }

  async function rerunOcr() {
    const dId = dossierId.trim();
    const docId = documentId.trim();
    if (!credential.current || !dId || !docId || !version.trim() || !reason.trim()) return;
    setBusy(true);
    setStatus('Đang gửi yêu cầu OCR...');
    try {
      const res = await fetch(`/v1/dossiers/${encodeURIComponent(dId)}/documents/${encodeURIComponent(docId)}/ocr`, {
        method: 'POST',
        headers: {
          Authorization: 'Bearer ' + credential.current,
          'Content-Type': 'application/json',
          'Idempotency-Key': 'ui-ocr-' + crypto.randomUUID(),
          'If-Finance-Version': version.trim(),
        },
        body: JSON.stringify({ reason: reason.trim() }),
        cache: 'no-store',
        credentials: 'omit',
      });
      const nextVer = res.headers.get('finance-version');
      if (nextVer) setVersion(nextVer);
      if (!res.ok) {
        setStatus(`Yêu cầu OCR thất bại (${res.status}).`);
        return;
      }
      const data = await res.json() as Job;
      setJob(data);
      setStatus(`Đã xếp hàng OCR job ${data.id}.`);
    } catch {
      setStatus('Lỗi mạng khi gọi OCR.');
    } finally {
      setBusy(false);
    }
  }

  async function fetchEvidence() {
    const id = evidenceId.trim();
    if (!credential.current || !id) return;
    setBusy(true);
    try {
      const res = await fetch('/v1/evidence/' + encodeURIComponent(id), {
        headers: { Authorization: 'Bearer ' + credential.current },
        cache: 'no-store',
        credentials: 'omit',
      });
      if (!res.ok) {
        setEvidence(null);
        setStatus(`Không đọc được evidence (${res.status}).`);
        return;
      }
      const data = await res.json() as Evidence;
      setEvidence(data);
      setStatus(`Evidence ${data.id} tại ${data.locator}.`);
    } catch {
      setStatus('Lỗi mạng khi đọc evidence.');
    } finally {
      setBusy(false);
    }
  }

  return <section aria-labelledby="e06-title">
    <h2 id="e06-title">OCR &amp; Nguồn bằng chứng (E06)</h2>
    <div className="sandbox-connection">
      <label htmlFor="e06-token">Token Requester sandbox</label>
      <div className="actions">
        <input id="e06-token" type="password" autoComplete="off" value={token} onChange={e => setToken(e.target.value)} disabled={busy || connected} maxLength={16384} />
        <button className="primary" type="button" disabled={busy || connected || !token.trim()} onClick={() => { credential.current = token.trim(); setToken(''); setConnected(true); setStatus('Đã nạp token.'); }}>Kết nối</button>
        <button type="button" onClick={disconnect}>Ngắt kết nối</button>
      </div>
    </div>
    <div className="actions">
      <input value={dossierId} onChange={e => setDossierId(e.target.value)} placeholder="Dossier UUID" aria-label="Dossier UUID" />
      <input value={documentId} onChange={e => setDocumentId(e.target.value)} placeholder="Document UUID" aria-label="Document UUID" />
      <input value={version} onChange={e => setVersion(e.target.value)} placeholder="If-Finance-Version" aria-label="If-Finance-Version" />
      <input value={reason} onChange={e => setReason(e.target.value)} placeholder="Lý do OCR" aria-label="Lý do OCR" />
      <button type="button" disabled={!connected || busy || !dossierId.trim() || !documentId.trim() || !version.trim()} onClick={() => void rerunOcr()}>Chạy lại OCR</button>
    </div>
    <div className="actions">
      <input value={evidenceId} onChange={e => setEvidenceId(e.target.value)} placeholder="Evidence UUID" aria-label="Evidence UUID" />
      <button type="button" disabled={!connected || busy || !evidenceId.trim()} onClick={() => void fetchEvidence()}>Đọc Evidence</button>
    </div>
    <p role="status" aria-live="polite">{status}</p>
    {job && <p>Job {job.id}: {job.state} ({job.progress}%)</p>}
    {evidence && <p>Locator: {evidence.locator} | Page: {evidence.page ?? '-'} | Excerpt: {evidence.excerpt ?? '-'}</p>}
  </section>;
}
