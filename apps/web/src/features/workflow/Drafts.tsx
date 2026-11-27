import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';
import { DocumentIntents } from './DocumentIntents';

type Dossier = components['schemas']['Dossier'];
type Kind = Dossier['kind'];
type Page = { items: Dossier[]; next_cursor?: string };
const kinds: Record<Kind,string> = { PAYMENT:'Thanh toán', ADVANCE:'Tạm ứng', SETTLEMENT:'Quyết toán tạm ứng' };
type Pending = { key: string; company: string; kind: Kind };

export function Drafts() {
  const [token, setToken] = useState('');
  const [connected, setConnected] = useState(false);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState('Chưa kết nối.');
  const [company, setCompany] = useState('');
  const [kind, setKind] = useState<Kind>('PAYMENT');
  const [items, setItems] = useState<Dossier[]>([]);
  const [cursor, setCursor] = useState<string>();
  const [detail, setDetail] = useState<Dossier>();
  const [uncertain, setUncertain] = useState(false);
  const credential = useRef('');
  const current = useRef<AbortController | null>(null);
  const generation = useRef(0);
  const pending = useRef<Pending | null>(null);
  const active = useRef(false);
  useEffect(() => () => { generation.current++; current.current?.abort(); credential.current=''; pending.current=null; }, []);

  function clear() {
    generation.current++;
    current.current?.abort();
    active.current = false;
    credential.current = '';
    pending.current = null;
    setToken(''); setConnected(false); setBusy(false); setItems([]); setCursor(undefined); setDetail(undefined); setUncertain(false);
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
      const params=new URLSearchParams({limit:'10'}); if(next)params.set('cursor',next);
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
  return <section aria-labelledby="draft-title">
    <h2 id="draft-title">Hồ sơ tài chính nháp</h2>
    <p>Sandbox — chỉ dữ liệu tổng hợp. Chưa mở tải chứng từ, trình duyệt, phê duyệt hoặc thanh toán. Token không lưu vào trình duyệt.</p>
    <label htmlFor="draft-token">Token sandbox</label>
    <div className="actions"><input id="draft-token" type="password" autoComplete="off" maxLength={16384} value={token} disabled={busy||connected} onChange={e=>setToken(e.target.value)} />
      <button className="primary" disabled={busy||connected||!token.trim()} onClick={()=>{credential.current=token.trim();setToken('');void list();}}>Kết nối</button>
      <button disabled={busy||uncertain} onClick={clear}>Ngắt kết nối</button>
    </div>
    <div className="draft-controls">
      <div><label htmlFor="company">UUID đơn vị sandbox</label><input id="company" value={company} disabled={!connected||busy||uncertain} maxLength={36} onChange={e=>setCompany(e.target.value)}/></div>
      <div><label htmlFor="kind">Loại hồ sơ</label><select id="kind" value={kind} disabled={!connected||busy||uncertain} onChange={e=>setKind(e.target.value as Kind)}>{Object.entries(kinds).map(([k,v])=><option key={k} value={k}>{v}</option>)}</select></div>
      <button className="primary" disabled={!connected||busy||(!uncertain&&!/^[0-9a-f-]{36}$/i.test(company.trim()))} onClick={()=>void create()}>{uncertain?'Thử lại yêu cầu':'Tạo hồ sơ nháp'}</button>
    </div>
    <p role="status" aria-live="polite">{status}</p>
    <div className="table-wrap"><table><thead><tr><th>Mã hồ sơ</th><th>Loại</th><th>Trạng thái</th><th>Phiên bản</th><th>Chi tiết</th></tr></thead>
      <tbody>{items.map(row=><tr key={row.id}><td>{row.code}</td><td>{kinds[row.kind]}</td><td>{row.status}</td><td>{row.version}</td><td><button disabled={busy} onClick={()=>void view(row.id)}>Xem</button></td></tr>)}</tbody></table></div>
    <div className="actions"><button disabled={!connected||busy} onClick={()=>void list()}>Trang đầu</button><button disabled={!cursor||busy} onClick={()=>void list(cursor)}>Trang tiếp</button></div>
    {detail&&<div className="draft-detail"><h3>Chi tiết hồ sơ</h3><dl><dt>Mã hồ sơ</dt><dd>{detail.code}</dd><dt>Loại hồ sơ</dt><dd>{kinds[detail.kind]}</dd><dt>Trạng thái</dt><dd>{detail.status}</dd><dt>Ngày cập nhật UTC</dt><dd>{detail.updated_at}</dd></dl>{connected&&<DocumentIntents key={detail.id} dossier={detail} token={credential.current}/>}</div>}
  </section>;
}
