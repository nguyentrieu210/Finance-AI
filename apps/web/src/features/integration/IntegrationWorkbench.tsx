import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';

type Handoff = components['schemas']['Handoff'];
type Reconciliation = components['schemas']['Reconciliation'];
type Dossier = components['schemas']['Dossier'];
type Payment = components['schemas']['PaymentStatus'];
type Command = { key: string; dossier: string; version: number; packageId: string; external: string; evidence: string[] };
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function IntegrationWorkbench() {
  const [token, setToken] = useState('');
  const [connected, setConnected] = useState(false);
  const [dossier, setDossier] = useState('');
  const [packageId, setPackageId] = useState('');
  const [external, setExternal] = useState('');
  const [evidence, setEvidence] = useState('');
  const [handoffs, setHandoffs] = useState<Handoff[]>([]);
  const [cases, setCases] = useState<Reconciliation[]>([]);
  const [payment, setPayment] = useState<Payment>();
  const [current, setCurrent] = useState<Dossier>();
  const [status, setStatus] = useState('Chưa kết nối.');
  const [busy, setBusy] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const credential = useRef('');
  const pending = useRef<Command | null>(null);
  const request = useRef<AbortController | null>(null);
  useEffect(() => () => { request.current?.abort(); credential.current = ''; pending.current = null; }, []);
  function clear() {
    request.current?.abort(); credential.current = ''; pending.current = null;
    setConnected(false); setToken(''); setHandoffs([]); setCases([]); setPayment(undefined); setCurrent(undefined);
    setBusy(false); setUncertain(false); setStatus('Đã ngắt kết nối.');
  }
  async function api(path: string, controller: AbortController, init: RequestInit = {}) {
    return fetch(path, { ...init, headers: { ...init.headers, Authorization: 'Bearer ' + credential.current }, cache: 'no-store', credentials: 'omit', signal: controller.signal });
  }
  async function load() {
    if (!credential.current || !uuid.test(dossier.trim())) { setStatus('Nhập UUID hồ sơ hợp lệ.'); return; }
    const controller = new AbortController(); request.current?.abort(); request.current = controller;
    setBusy(true); setCurrent(undefined); setPayment(undefined); setHandoffs([]); setCases([]);
    try {
      const base = '/v1/dossiers/' + encodeURIComponent(dossier.trim());
      const [d, h, p, r] = await Promise.all([api(base, controller), api(base + '/handoffs?limit=20', controller), api(base + '/payment-status', controller), api('/v1/reconciliation?limit=20', controller)]);
      if (controller.signal.aborted) return;
      if (!d.ok || !h.ok || !p.ok) throw new Error('read denied');
      const actual = await d.json() as Dossier;
      const pay = await p.json() as Payment;
      const rows = await h.json() as { items: Handoff[] };
      const reconciliations = r.ok ? await r.json() as { items: Reconciliation[] } : { items: [] };
      if (controller.signal.aborted) return;
      if (actual.id !== dossier.trim() || pay.dossier_id !== actual.id) throw new Error('invalid scope');
      setCurrent(actual); setPayment(pay); setHandoffs(rows.items); setCases(reconciliations.items);
      setConnected(true); setStatus('Đã tải hồ sơ thực từ API. Bàn giao không đồng nghĩa thanh toán.');
    } catch { if (!controller.signal.aborted) setStatus('Không tải được hồ sơ hoặc không có quyền.'); }
    finally { if (!controller.signal.aborted) setBusy(false); }
  }
  const evidenceIds = evidence.split(/[\s,;]+/).filter(Boolean);
  const valid = Boolean(current?.status === 'EXPORTED' && uuid.test(packageId.trim()) && external.trim() && evidenceIds.length <= 64 && evidenceIds.every(id => uuid.test(id)) && new Set(evidenceIds).size === evidenceIds.length);
  async function handoff() {
    if (busy || !credential.current || (!pending.current && (!valid || !current))) return;
    if (!pending.current && current) pending.current = { key: crypto.randomUUID(), dossier: current.id, version: current.version, packageId: packageId.trim(), external: external.trim(), evidence: evidenceIds };
    const cmd = pending.current;
    if (!cmd) return;
    const controller = new AbortController(); request.current?.abort(); request.current = controller;
    setBusy(true);
    try {
      const response = await api('/v1/dossiers/' + encodeURIComponent(cmd.dossier) + '/handoffs', controller, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'Idempotency-Key': cmd.key, 'If-Finance-Version': '"v1:dossier:' + cmd.dossier + ':' + cmd.version + '"' },
        body: JSON.stringify({ package_id: cmd.packageId, channel: 'MANUAL', external_ref: cmd.external, evidence_ids: cmd.evidence }),
      });
      if (controller.signal.aborted) return;
      if (!response.ok) {
        if (response.status >= 500) { setUncertain(true); setStatus('Chưa rõ kết quả. Thử lại đúng yêu cầu cũ.'); }
        else { pending.current = null; setUncertain(false); setStatus('Bàn giao bị từ chối (' + response.status + ').'); }
        return;
      }
      const result = await response.json() as Handoff;
      if (controller.signal.aborted) return;
      if (result.package_id !== cmd.packageId || result.external_ref !== cmd.external || result.state !== 'ACKNOWLEDGED') throw new Error('invalid receipt');
      pending.current = null; setUncertain(false); setExternal(''); setEvidence(''); setBusy(false);
      await load();
    } catch { if (!controller.signal.aborted) { setUncertain(true); setStatus('Chưa rõ kết quả. Thử lại đúng yêu cầu cũ.'); } }
    finally { if (!controller.signal.aborted) setBusy(false); }
  }
  return <section>
    <h2>Bàn giao và đối soát</h2>
    <p>DO/ERP live chưa được cấp quyền. Chỉ ghi nhận mã biên nhận có thật; không tự sinh mã bên ngoài.</p>
    <label>Token sandbox<input type="password" autoComplete="off" value={token} disabled={connected || busy} onChange={e => setToken(e.target.value)}/></label>
    <label>UUID hồ sơ<input value={dossier} disabled={busy || uncertain} onChange={e => { setDossier(e.target.value); setCurrent(undefined); setPayment(undefined); setHandoffs([]); setCases([]); }}/></label>
    <div className="actions">
      <button disabled={busy || uncertain || !uuid.test(dossier.trim()) || (!connected && !token.trim())} onClick={() => { if (!connected) { credential.current = token.trim(); setToken(''); } void load(); }}>{connected ? 'Tải hồ sơ' : 'Kết nối'}</button>
      <button disabled={busy || uncertain} onClick={clear}>Ngắt</button>
    </div>
    <p role="status">{status}</p>
    {current && <p>Trạng thái hồ sơ: {current.status}. Phiên bản {current.version}.</p>}
    {payment && <p>Thanh toán: {payment.state === 'UNKNOWN' ? 'Chưa xác minh được' : payment.state}. {payment.confirmed_paid ? 'Có số tiền đã xác minh.' : 'Chưa có số tiền được xác minh.'}</p>}
    <label>UUID gói đã xuất<input value={packageId} disabled={busy || uncertain} onChange={e => setPackageId(e.target.value)}/></label>
    <label>Mã biên nhận bên ngoài<input value={external} maxLength={256} disabled={busy || uncertain} onChange={e => setExternal(e.target.value)}/></label>
    <label>UUID bằng chứng (nếu có, phân cách bằng dấu phẩy)<textarea value={evidence} disabled={busy || uncertain} onChange={e => setEvidence(e.target.value)}/></label>
    <button disabled={!connected || busy || (!uncertain && !valid)} onClick={() => void handoff()}>{uncertain ? 'Thử lại bàn giao' : 'Ghi nhận bàn giao MANUAL'}</button>
    <h3>Biên nhận bàn giao</h3><div className="table-wrap"><table><thead><tr><th>Kênh</th><th>Trạng thái</th><th>Mã bên ngoài</th></tr></thead><tbody>{handoffs.map(x => <tr key={x.id}><td>{x.channel}</td><td>{x.state}</td><td>{x.external_ref ?? ''}</td></tr>)}</tbody></table></div>
    <h3>Đối soát</h3><div className="table-wrap"><table><thead><tr><th>Kênh</th><th>Trạng thái</th><th>Lý do</th></tr></thead><tbody>{cases.map(x => <tr key={x.id}><td>{x.channel}</td><td>{x.state}</td><td>{x.reason}</td></tr>)}</tbody></table></div>
  </section>;
}
