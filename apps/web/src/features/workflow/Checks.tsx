import { useEffect, useRef, useState } from 'react';
import { EvidenceViewer } from './EvidenceViewer';
import type { components } from '../../generated/api';

type Check = components['schemas']['CheckResult'];
type Page = { items: Check[]; next_cursor?: string };

export function Checks({ dossierId, version, token }: { dossierId: string; version: number; token: string }) {
  const [items, setItems] = useState<Check[]>([]);
  const [cursor, setCursor] = useState<string>();
  const [requested, setRequested] = useState<string>();
  const [generation, setGeneration] = useState(0);
  const [message, setMessage] = useState('Chưa tải kết quả kiểm tra.');
  const [busy, setBusy] = useState(false);
  const [selectedEvidence, setSelectedEvidence] = useState<string>();
  const [selectedWarning, setSelectedWarning] = useState<string>();
  const [reason, setReason] = useState('');
  const [acknowledged, setAcknowledged] = useState<string[]>([]);
  const [ackBusy, setAckBusy] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const pending = useRef<{ id: string; reason: string; key: string } | null>(null);
  const ackAbort = useRef<AbortController | null>(null);
  useEffect(() => () => { ackAbort.current?.abort(); pending.current = null; }, [dossierId, version, token]);

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
    setSelectedEvidence(undefined); setRequested(next); setGeneration(n => n + 1);
  }

  async function acknowledge() {
    if (ackBusy || busy || !selectedWarning || !reason.trim()) return;
    if (!pending.current) pending.current = { id: selectedWarning, reason: reason.trim(), key: crypto.randomUUID() };
    const cmd = pending.current;
    const abort = new AbortController();
    ackAbort.current = abort; setAckBusy(true);
    try {
      const response = await fetch('/v1/dossiers/' + encodeURIComponent(dossierId) + '/acknowledge', {
        method: 'POST', headers: {
          Authorization: 'Bearer ' + token, 'Content-Type': 'application/json',
          'Idempotency-Key': cmd.key, 'If-Finance-Version': '"v1:dossier:' + dossierId + ':' + version + '"',
        },
        body: JSON.stringify({ result_id: cmd.id, reason: cmd.reason }),
        cache: 'no-store', credentials: 'omit', signal: abort.signal,
      });
      if (abort.signal.aborted) return;
      if (!response.ok) {
        if (response.status >= 500) { setUncertain(true); setMessage('Chưa rõ kết quả xác nhận. Thử lại cùng yêu cầu.'); }
        else { pending.current = null; setUncertain(false); setMessage('Không xác nhận được cảnh báo (' + response.status + ').'); }
        return;
      }
      const receipt = await response.json() as Check;
      if (abort.signal.aborted) return;
      if (receipt.id !== cmd.id || receipt.outcome !== 'FAIL' || receipt.severity !== 'WARNING') throw new Error('invalid receipt');
      pending.current = null; setUncertain(false); setSelectedWarning(undefined); setReason('');
      setAcknowledged(ids => [...ids, cmd.id]);
      setMessage('Đã lưu xác nhận cảnh báo; kết quả vẫn là FAIL, không phải phê duyệt.');
    } catch {
      if (!abort.signal.aborted) { setUncertain(true); setMessage('Chưa rõ kết quả xác nhận. Thử lại cùng yêu cầu.'); }
    } finally { if (!abort.signal.aborted) setAckBusy(false); }
  }

  return <section aria-label="Kết quả kiểm tra hồ sơ">
    <h4>Kết quả kiểm tra hồ sơ</h4>
    <p>Chỉ hiển thị chẩn đoán sandbox đã lưu, không phải phê duyệt tài chính.</p>
    <div className="actions"><button disabled={busy || ackBusy || uncertain} onClick={() => load()}>Tải kết quả kiểm tra</button>
      <button disabled={busy || ackBusy || uncertain || !cursor} onClick={() => load(cursor)}>Trang kiểm tra tiếp</button></div>
    <p role="status">{message}</p>
    {items.length > 0 && <div className="table-wrap"><table>
      <thead><tr><th>Quy tắc</th><th>Kết quả</th><th>Mức độ</th><th>Mã lý do</th><th>Bằng chứng</th><th>Xác nhận</th></tr></thead>
      <tbody>{items.map(row => <tr key={row.id}><td>{row.rule_id}</td><td>{row.outcome}</td><td>{row.severity}</td><td>{row.reason_code}</td><td>{row.evidence_ids.map((id, index) => <button type="button" key={id} onClick={() => setSelectedEvidence(id)}>Xem {index + 1}</button>)}</td><td>{row.severity === 'WARNING' && row.outcome === 'FAIL' && (acknowledged.includes(row.id) ? 'Đã xác nhận' : <button type="button" disabled={busy || ackBusy || uncertain} onClick={() => { setSelectedWarning(row.id); setReason(''); }}>Xác nhận cảnh báo</button>)}</td></tr>)}</tbody>
    </table></div>}
    {selectedWarning && <section aria-label="Xác nhận cảnh báo">
      <p>Xác nhận đã đọc cảnh báo không thay thế kiểm tra bắt buộc.</p>
      <label>Lý do<textarea maxLength={512} value={reason} disabled={ackBusy || uncertain} onChange={e => setReason(e.target.value)}/></label>
      <button type="button" disabled={busy || ackBusy || !reason.trim()} onClick={() => void acknowledge()}>{uncertain ? 'Thử lại xác nhận' : 'Lưu xác nhận'}</button>
      <button type="button" disabled={ackBusy || uncertain} onClick={() => { setSelectedWarning(undefined); setReason(''); }}>Đóng</button>
    </section>}
    {selectedEvidence && <EvidenceViewer key={selectedEvidence} id={selectedEvidence} token={token} onClose={() => setSelectedEvidence(undefined)}/>}
  </section>;
}
