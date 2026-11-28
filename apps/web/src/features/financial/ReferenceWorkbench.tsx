import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';
import { VerifiedContracts } from '../contracts/VerifiedContracts';
import { VerifiedAdvances } from '../advances/VerifiedAdvances';
import { VerifiedBudgets } from '../budget/VerifiedBudgets';

type Job = components['schemas']['Job'];
type Reference = components['schemas']['Reference'];

const defaultKind: Record<'E12' | 'E13' | 'E14', Reference['kind']> = {
  E12: 'CONTRACT',
  E13: 'ADVANCE',
  E14: 'BUDGET',
};

export function ReferenceWorkbench({ module }: { module: 'E12' | 'E13' | 'E14' }) {
  const [token, setToken] = useState('');
  const [activeToken, setActiveToken] = useState('');
  const [documentId, setDocumentId] = useState('00000000-0000-0000-0000-000000001a02');
  const [kind, setKind] = useState<Reference['kind']>(defaultKind[module]);
  const [coverage, setCoverage] = useState<Reference['coverage']>('COMPLETE');
  const [refId, setRefId] = useState('');
  const [evidenceId, setEvidenceId] = useState('00000000-0000-0000-0000-000000002333');
  const [reason, setReason] = useState('Xác minh tham chiếu từ chứng từ nguồn');
  const [job, setJob] = useState<Job | null>(null);
  const [verifiedRef, setVerifiedRef] = useState<Reference | null>(null);
  const [status, setStatus] = useState('Chưa kết nối.');
  const [busy, setBusy] = useState(false);
  const credential = useRef('');

  useEffect(() => {
    setKind(defaultKind[module]);
  }, [module]);

  useEffect(() => () => {
    credential.current = '';
  }, []);

  function disconnect() {
    credential.current = '';
    setToken('');
    setActiveToken('');
    setJob(null);
    setVerifiedRef(null);
    setStatus('Đã ngắt kết nối.');
  }

  async function createReference() {
    if (!credential.current || !documentId.trim()) return;
    setBusy(true);
    setStatus('Đang tạo bản ghi tham chiếu...');
    try {
      const res = await fetch('/v1/references', {
        method: 'POST',
        headers: {
          Authorization: 'Bearer ' + credential.current,
          'Content-Type': 'application/json',
          'Idempotency-Key': 'ui-ref-create-' + crypto.randomUUID(),
        },
        body: JSON.stringify({
          document_id: documentId.trim(),
          kind,
          as_of: new Date().toISOString(),
          coverage,
        }),
        cache: 'no-store',
        credentials: 'omit',
      });
      if (!res.ok) {
        setStatus(`Tạo tham chiếu thất bại (${res.status}).`);
        return;
      }
      const data = await res.json() as Job;
      setJob(data);
      setRefId(data.id);
      setStatus(`Đã tạo reference ${data.id} (PENDING).`);
    } catch {
      setStatus('Lỗi mạng khi tạo tham chiếu.');
    } finally {
      setBusy(false);
    }
  }

  async function verifyReference() {
    const id = refId.trim();
    const ev = evidenceId.trim();
    if (!credential.current || !id || !ev || !reason.trim()) return;
    setBusy(true);
    setStatus(`Đang đọc phiên bản và xác minh ${id}...`);
    try {
      const getRes = await fetch('/v1/references/' + encodeURIComponent(id), {
        headers: { Authorization: 'Bearer ' + credential.current },
        cache: 'no-store',
        credentials: 'omit',
      });
      const ver = getRes.headers.get('finance-version');
      if (!getRes.ok || !ver) {
        setStatus(`Không lấy được Finance-Version tham chiếu (${getRes.status}).`);
        return;
      }
      const res = await fetch(`/v1/references/${encodeURIComponent(id)}/verify`, {
        method: 'POST',
        headers: {
          Authorization: 'Bearer ' + credential.current,
          'Content-Type': 'application/json',
          'Idempotency-Key': 'ui-ref-verify-' + crypto.randomUUID(),
          'If-Finance-Version': ver,
        },
        body: JSON.stringify({ reason: reason.trim(), evidence_ids: [ev] }),
        cache: 'no-store',
        credentials: 'omit',
      });
      if (!res.ok) {
        setStatus(`Xác minh tham chiếu thất bại (${res.status}).`);
        return;
      }
      const data = await res.json() as Reference;
      setVerifiedRef(data);
      setStatus(`Đã xác minh ${data.id}: ${data.state}.`);
    } catch {
      setStatus('Lỗi mạng khi xác minh tham chiếu.');
    } finally {
      setBusy(false);
    }
  }

  return <section aria-labelledby="ref-wb-title">
    <h2 id="ref-wb-title">Quản trị Tham chiếu Tài chính ({module})</h2>
    <div className="sandbox-connection">
      <label htmlFor="ref-wb-token">Token sandbox</label>
      <div className="actions">
        <input id="ref-wb-token" type="password" autoComplete="off" value={token} onChange={e => setToken(e.target.value)} disabled={busy || Boolean(activeToken)} maxLength={16384} />
        <button className="primary" type="button" disabled={busy || Boolean(activeToken) || !token.trim()} onClick={() => {
          const clean = token.trim();
          credential.current = clean;
          setToken('');
          setActiveToken(clean);
          setStatus('Đã nạp token.');
        }}>Kết nối</button>
        <button type="button" onClick={disconnect}>Ngắt kết nối</button>
      </div>
    </div>
    {activeToken && <>
      {module === 'E12' && <VerifiedContracts token={activeToken} />}
      {module === 'E13' && <VerifiedAdvances token={activeToken} />}
      {module === 'E14' && <VerifiedBudgets token={activeToken} />}
      <div className="actions">
        <input value={documentId} onChange={e => setDocumentId(e.target.value)} placeholder="Document UUID" aria-label="Document UUID" />
        <select value={kind} onChange={e => setKind(e.target.value as Reference['kind'])} aria-label="Loại tham chiếu">
          <option value="CONTRACT">CONTRACT</option>
          <option value="ADVANCE">ADVANCE</option>
          <option value="BUDGET">BUDGET</option>
          <option value="VENDOR">VENDOR</option>
          <option value="INVOICE">INVOICE</option>
          <option value="PAYMENT">PAYMENT</option>
        </select>
        <select value={coverage} onChange={e => setCoverage(e.target.value as Reference['coverage'])} aria-label="Coverage">
          <option value="COMPLETE">COMPLETE</option>
          <option value="PARTIAL">PARTIAL</option>
          <option value="UNKNOWN">UNKNOWN</option>
        </select>
        <button type="button" disabled={busy} onClick={() => void createReference()}>Tạo Reference</button>
      </div>
      <div className="actions">
        <input value={refId} onChange={e => setRefId(e.target.value)} placeholder="Reference UUID" aria-label="Reference UUID" />
        <input value={evidenceId} onChange={e => setEvidenceId(e.target.value)} placeholder="Evidence UUID" aria-label="Evidence UUID" />
        <input value={reason} onChange={e => setReason(e.target.value)} placeholder="Lý do verify" aria-label="Lý do verify" />
        <button type="button" disabled={busy || !refId.trim()} onClick={() => void verifyReference()}>Xác minh (Approver)</button>
      </div>
    </>}
    <p role="status" aria-live="polite">{status}</p>
    {job && <p>Reference job: {job.id} ({job.state})</p>}
    {verifiedRef && <p>Đã xác minh {verifiedRef.kind}: {verifiedRef.source_key} ({verifiedRef.state})</p>}
  </section>;
}
