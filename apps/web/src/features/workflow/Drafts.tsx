import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';
import { DocumentIntents } from './DocumentIntents';
import { Checks } from './Checks';
import { Checklist } from './Checklist';
import { VerifiedContracts } from '../contracts/VerifiedContracts';
import { VerifiedAdvances } from '../advances/VerifiedAdvances';
import { VerifiedBudgets } from '../budget/VerifiedBudgets';
import { Fields } from '../extraction/Fields';

type Dossier = components['schemas']['Dossier'];
type Kind = Dossier['kind'];
type Page = { items: Dossier[]; next_cursor?: string };
const kinds: Record<Kind,string> = { PAYMENT:'Thanh toán', ADVANCE:'Tạm ứng', SETTLEMENT:'Quyết toán tạm ứng' };
type Pending = { key: string; company: string; kind: Kind };
type ChangeOp = 'revise' | 'reopen' | 'cancel';
type PendingChange = { key: string; id: string; version: number; operation: ChangeOp; reason: string };
const changeLabels: Record<ChangeOp, string> = {
  revise: 'Tạo phiên bản mới', reopen: 'Mở lại hồ sơ', cancel: 'Hủy hồ sơ',
};
function allowedChanges(status: Dossier['status']): ChangeOp[] {
  switch (status) {
    case 'DRAFT': case 'NEEDS_INPUT': case 'REVIEWABLE': return ['revise', 'cancel'];
    case 'RETURNED': return ['reopen', 'cancel'];
    default: return [];
  }
}

export function Drafts() {
  const [token, setToken] = useState('');
  const [connected, setConnected] = useState(false);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('Chưa kết nối.');
  const [company, setCompany] = useState('');
  const [kind, setKind] = useState<Kind>('PAYMENT');
  const [filterKind, setFilterKind] = useState<Kind | ''>('');
  const [items, setItems] = useState<Dossier[]>([]);
  const [cursor, setCursor] = useState<string>();
  const [detail, setDetail] = useState<Dossier>();
  const [uncertain, setUncertain] = useState(false);
  const [changeUncertain, setChangeUncertain] = useState(false);
  const [changeReason, setChangeReason] = useState('');
  const [changeOp, setChangeOp] = useState<ChangeOp>();
  const pendingChange = useRef<PendingChange | null>(null);
  const credential = useRef('');
  const current = useRef<AbortController | null>(null);
  const generation = useRef(0);
  const pending = useRef<Pending | null>(null);
  const active = useRef(false);
  useEffect(() => () => { generation.current++; current.current?.abort(); credential.current=''; pending.current=null; pendingChange.current=null; }, []);

  function clear() {
    generation.current++;
    current.current?.abort();
    active.current = false;
    credential.current = '';
    pending.current = null;
    pendingChange.current = null; setChangeUncertain(false); setChangeReason(''); setChangeOp(undefined);
    setFilterKind(''); setToken(''); setConnected(false); setBusy(false); setItems([]); setCursor(undefined); setDetail(undefined); setUncertain(false);
    setStatus('Đã ngắt kết nối.');
  }
  function start() {
    if (active.current || !credential.current) return null;
    active.current=true; setBusy(true);
    const controller = new AbortController();
    current.current=controller;
    return { controller, version: ++generation.current };
  }
  function finish(version:number) {
    if (version===generation.current) {active.current=false;setBusy(false);}
  }
  async function fetchData(path:string, controller:AbortController, options:RequestInit={}) {
    return fetch(path,{ ...options, headers:{...options.headers,Authorization:'Bearer '+credential.current}, cache:'no-store',credentials:'omit',signal:controller.signal });
  }
  async function list(next?:string) {
    const op=start(); if(!op)return;
    setStatus('Đang tải hồ sơ...');
    try {
      const params=new URLSearchParams({limit:'10'}); if(filterKind)params.set('kind',filterKind); if(next)params.set('cursor',next);
      const response=await fetchData('/v1/dossiers?'+params,op.controller);
      if(op.version!==generation.current)return;
      if(!response.ok) {
        if(response.status===401 || response.status===403){credential.current='';setConnected(false);}
        setItems([]);setCursor(undefined);setDetail(undefined);setStatus('Không thể tải hồ sơ ('+response.status+').');return;
      }
      const result=await response.json() as Page;
      if(op.version!==generation.current)return;
      if(!Array.isArray(result.items))throw new Error('Invalid response');
      setItems(result.items);setCursor(result.next_cursor);setConnected(true);setStatus('Đã tải '+result.items.length+' hồ sơ từ API.');
    } catch {if(op.version===generation.current&&!op.controller.signal.aborted)setStatus('Không kết nối được API.');}
    finally {finish(op.version);}
  }
  async function create() {
    const op=start(); if(!op)return;
    if(!pending.current)pending.current={key:crypto.randomUUID(),company:company.trim(),kind};
    const cmd=pending.current;
    setStatus('Đang tạo hồ sơ nháp...');
    try {
      const response=await fetchData('/v1/dossiers',op.controller,{
        method:'POST',headers:{'Content-Type':'application/json','Idempotency-Key':cmd.key},
        body:JSON.stringify({company_id:cmd.company,kind:cmd.kind}),
      });
      if(op.version!==generation.current)return;
      if(!response.ok) {
        if(response.status===401 || response.status===403){credential.current='';setConnected(false);}
        if(response.status>=500) {
          setUncertain(true);setStatus('Chưa rõ kết quả. Thử lại cùng yêu cầu, không tạo mã mới.');
        } else {
          pending.current=null;setUncertain(false);setStatus('Không tạo được hồ sơ ('+response.status+').');
        }
        return;
      }
      const draft=await response.json() as Dossier;
      if(op.version!==generation.current)return;
      if(!draft.id || draft.status!=='DRAFT')throw new Error('Invalid draft response');
      pending.current=null;setUncertain(false);setDetail(draft);
      setStatus('Đã tạo hồ sơ nháp '+draft.code+'.');
      finish(op.version);
      await list();
    } catch {
      if(op.version===generation.current&&!op.controller.signal.aborted){setUncertain(true);setStatus('Chưa rõ kết quả. Thử lại cùng yêu cầu, không tạo mã mới.');}
    } finally {finish(op.version);}
  }
  async function view(id:string) {
    const op=start();if(!op)return;
    try{
      const response=await fetchData('/v1/dossiers/'+encodeURIComponent(id),op.controller);
      if(op.version!==generation.current)return;
      if(!response.ok){
        if(response.status===401 || response.status===403){credential.current='';setConnected(false);setItems([]);}
        setDetail(undefined);setStatus('Không xem được hồ sơ ('+response.status+').');return;
      }
      const draft=await response.json() as Dossier;
      if(op.version===generation.current){setDetail(draft);setStatus('Đã tải chi tiết hồ sơ.');}
    }catch{if(op.version===generation.current&&!op.controller.signal.aborted)setStatus('Không kết nối được API.');}
    finally{finish(op.version);}
  }
  async function changeDossier() {
    if (!detail || uncertain || (!pendingChange.current && (!changeOp || !allowedChanges(detail.status).includes(changeOp) || !changeReason.trim()))) return;
    const op = start(); if (!op) return;
    if (!pendingChange.current && changeOp) pendingChange.current = {
      key: crypto.randomUUID(), id: detail.id, version: detail.version,
      operation: changeOp, reason: changeReason.trim(),
    };
    const cmd = pendingChange.current;
    if (!cmd) { finish(op.version); return; }
    try {
      const response = await fetchData('/v1/dossiers/' + encodeURIComponent(cmd.id) + '/' + cmd.operation, op.controller, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json', 'Idempotency-Key': cmd.key,
          'If-Finance-Version': '"v1:dossier:' + cmd.id + ':' + cmd.version + '"',
        },
        body: JSON.stringify({ reason: cmd.reason, evidence_ids: [] }),
      });
      if (op.version !== generation.current) return;
      if (!response.ok) {
        if (response.status >= 500) {
          setChangeUncertain(true);
          setStatus('Chưa rõ kết quả thay đổi. Thử lại đúng yêu cầu cũ.');
        } else {
          pendingChange.current = null; setChangeUncertain(false);
          setStatus('Không thể thay đổi hồ sơ (' + response.status + '). Kiểm tra trạng thái, quyền và các khoản giữ chỗ.');
          if (response.status === 401 || response.status === 403) {
            credential.current = ''; setConnected(false); setItems([]); setDetail(undefined);
          }
        }
        return;
      }
      const result = await response.json() as Dossier;
      if (op.version !== generation.current) return;
      if (result.id !== cmd.id || result.version !== cmd.version + 1) throw new Error('Invalid change receipt');
      pendingChange.current = null; setChangeUncertain(false); setChangeReason(''); setChangeOp(undefined);
      // A replay may return an older command receipt, never present it as a fresh state.
      const latest = await fetchData('/v1/dossiers/' + encodeURIComponent(cmd.id), op.controller);
      if (op.version !== generation.current) return;
      if (!latest.ok) { setDetail(undefined); setStatus('Đã nhận biên nhận; hãy tải lại hồ sơ hiện tại.'); return; }
      const actual = await latest.json() as Dossier;
      if (op.version !== generation.current) return;
      if (actual.id !== cmd.id || actual.version < result.version) throw new Error('Invalid current dossier');
      setDetail(actual); setItems(rows => rows.map(row => row.id === actual.id ? actual : row));
      setStatus('Đã thực hiện ' + changeLabels[cmd.operation].toLowerCase() + '. Phiên bản hiện tại: ' + actual.version + '.');
    } catch {
      if (op.version === generation.current && !op.controller.signal.aborted) {
        setChangeUncertain(Boolean(pendingChange.current));
        if (!pendingChange.current) setDetail(undefined);
        setStatus(pendingChange.current ? 'Chưa rõ kết quả. Thử lại đúng yêu cầu cũ.' : 'Đã nhận biên nhận; hãy tải lại hồ sơ.');
      }
    } finally { finish(op.version); }
  }
  return <section aria-labelledby="draft-title">
    <h2 id="draft-title">Hồ sơ tài chính nháp</h2>
    <p>Sandbox — chỉ dữ liệu tổng hợp. Chưa mở tải chứng từ, trình duyệt, phê duyệt hoặc thanh toán. Token không lưu vào trình duyệt.</p>
    <label htmlFor="draft-token">Token sandbox</label>
    <div className="actions"><input id="draft-token" type="password" autoComplete="off" maxLength={16384} value={token} disabled={busy||connected} onChange={e=>setToken(e.target.value)} />
      <button className="primary" disabled={busy||connected||!token.trim()} onClick={()=>{credential.current=token.trim();setToken('');void list();}}>Kết nối</button>
      <button disabled={busy||uncertain||changeUncertain} onClick={clear}>Ngắt kết nối</button>
    </div>
    <div className="draft-controls">
      <div><label htmlFor="company">UUID đơn vị sandbox</label><input id="company" value={company} disabled={!connected||busy||uncertain||changeUncertain} maxLength={36} onChange={e=>setCompany(e.target.value)}/></div>
      <div><label htmlFor="kind">Loại hồ sơ</label><select id="kind" value={kind} disabled={!connected||busy||uncertain||changeUncertain} onChange={e=>setKind(e.target.value as Kind)}>{Object.entries(kinds).map(([k,v])=><option key={k} value={k}>{v}</option>)}</select></div>
      <button className="primary" disabled={!connected||busy||changeUncertain||(!uncertain&&!/^[0-9a-f-]{36}$/i.test(company.trim()))} onClick={()=>void create()}>{uncertain?'Thử lại yêu cầu':'Tạo hồ sơ nháp'}</button>
    </div>
    <p role="status" aria-live="polite">{status}</p>
    {connected&&<div className="reference-grid"><VerifiedContracts token={credential.current}/><VerifiedAdvances token={credential.current}/><VerifiedBudgets token={credential.current}/></div>}
    <div className="actions"><label htmlFor="filter-kind">Lọc loại hồ sơ</label><select id="filter-kind" value={filterKind} disabled={!connected||busy||changeUncertain} onChange={e=>{setFilterKind(e.target.value as Kind | '');setCursor(undefined);setItems([]);setDetail(undefined);setChangeOp(undefined);setChangeReason('');}}><option value="">Tất cả</option>{Object.entries(kinds).map(([k,v])=><option key={k} value={k}>{v}</option>)}</select><button disabled={!connected||busy||changeUncertain} onClick={()=>void list()}>Áp dụng bộ lọc</button></div>
    <div className="table-wrap"><table aria-label="Danh sách hồ sơ"><thead><tr><th>Mã hồ sơ</th><th>Loại</th><th>Trạng thái</th><th>Phiên bản</th><th>Chi tiết</th></tr></thead>
      <tbody>{items.map(row=><tr key={row.id}><td>{row.code}</td><td>{kinds[row.kind]}</td><td>{row.status}</td><td>{row.version}</td><td><button disabled={busy||changeUncertain} onClick={()=>{setChangeOp(undefined);setChangeReason('');void view(row.id);}}>Xem</button></td></tr>)}</tbody></table></div>
    <div className="actions"><button disabled={!connected||busy||changeUncertain} onClick={()=>void list()}>Trang đầu</button><button disabled={!cursor||busy||changeUncertain} onClick={()=>void list(cursor)}>Trang tiếp</button></div>
    {detail&&<div className="draft-detail"><h3>Chi tiết hồ sơ</h3><dl><dt>Mã hồ sơ</dt><dd>{detail.code}</dd><dt>Loại hồ sơ</dt><dd>{kinds[detail.kind]}</dd><dt>Trạng thái</dt><dd>{detail.status}</dd><dt>Ngày cập nhật UTC</dt><dd>{detail.updated_at}</dd></dl>{connected && (allowedChanges(detail.status).length > 0 || changeUncertain) && <section aria-label="Thay đổi vòng đời hồ sơ">
      <h4>Thay đổi vòng đời hồ sơ</h4>
      <p>Phiên bản {detail.version}. Không tự động giải phóng khoản giữ chỗ hoặc bỏ qua đối soát.</p>
      <label>Thao tác<select value={changeOp ?? ''} disabled={busy||uncertain||changeUncertain} onChange={e=>setChangeOp(e.target.value ? e.target.value as ChangeOp : undefined)}><option value="">Chọn thao tác</option>{allowedChanges(detail.status).map(op=><option key={op} value={op}>{changeLabels[op]}</option>)}</select></label>
      <label>Lý do<textarea maxLength={1024} value={changeReason} disabled={busy||uncertain||changeUncertain} onChange={e=>setChangeReason(e.target.value)}/></label>
      <button type="button" disabled={busy||uncertain||(!changeUncertain&&(!changeOp||!changeReason.trim()))} onClick={()=>void changeDossier()}>{changeUncertain?'Thử lại yêu cầu thay đổi':changeOp?changeLabels[changeOp]:'Thực hiện'}</button>
      {changeOp==='cancel'&&<p>Hủy là thao tác kết thúc hồ sơ; các bằng chứng cũ vẫn được lưu.</p>}
    </section>}
    {connected&&!changeUncertain&&detail.status!=='CANCELLED'&&<><DocumentIntents key={detail.id+':'+detail.version} dossier={detail} token={credential.current}/><Checks key={detail.id+':'+detail.version} dossierId={detail.id} version={detail.version} token={credential.current}/><Checklist key={detail.id+':checklist:'+detail.version} dossierId={detail.id} token={credential.current}/><Fields key={detail.id+':fields:'+detail.version} dossierId={detail.id} token={credential.current}/></>}</div>}
  </section>;
}
