import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';
type Audit = components['schemas']['Audit'];
type Page = { items: Audit[]; next_cursor?: string };

export function AuditLog() {
  const [token, setToken] = useState('');
  const [connected, setConnected] = useState(false);
  const [items, setItems] = useState<Audit[]>([]);
  const [cursor, setCursor] = useState<string>();
  const [status, setStatus] = useState('Chưa kết nối Auditor.');
  const [busy, setBusy] = useState(false);
  const credential = useRef('');
  const abort = useRef<AbortController | null>(null);
  useEffect(() => () => { abort.current?.abort(); credential.current=''; }, []);
  async function load(next?: string) {
    if (!credential.current || busy) return;
    abort.current?.abort(); const controller = new AbortController(); abort.current=controller;
    setBusy(true); setStatus('Đang đọc audit chain...');
    try {
      const query=new URLSearchParams({limit:'20'}); if(next)query.set('cursor',next);
      const response=await fetch('/v1/audit?'+query,{headers:{Authorization:'Bearer '+credential.current},cache:'no-store',credentials:'omit',signal:controller.signal});
      if(!response.ok){if(response.status===401||response.status===403){credential.current='';setConnected(false);}throw new Error(String(response.status));}
      const page=await response.json() as Page; if(!Array.isArray(page.items))throw new Error('invalid');
      setItems(page.items);setCursor(page.next_cursor);setConnected(true);setStatus('Đã đọc '+page.items.length+' audit event. Lượt đọc này được audit riêng.');
    } catch {if(!controller.signal.aborted){setItems([]);setCursor(undefined);setStatus('Không đọc được audit.');}}
    finally {if(!controller.signal.aborted)setBusy(false);}
  }
  function clear(){abort.current?.abort();credential.current='';setToken('');setConnected(false);setItems([]);setCursor(undefined);setBusy(false);setStatus('Đã ngắt Auditor.');}
  return <section aria-labelledby="audit-title"><h2 id="audit-title">Nhật ký kiểm toán</h2>
    <p>Chỉ tài khoản Auditor sandbox có quyền đọc. Không hiển thị raw token, chứng từ hoặc thông tin ngân hàng.</p>
    <label htmlFor="audit-token">Token Auditor sandbox</label><div className="actions">
      <input id="audit-token" type="password" autoComplete="off" maxLength={16384} value={token} disabled={busy||connected} onChange={e=>setToken(e.target.value)}/>
      <button className="primary" disabled={busy||connected||!token.trim()} onClick={()=>{credential.current=token.trim();setToken('');void load();}}>Kết nối</button>
      <button disabled={busy} onClick={clear}>Ngắt kết nối</button></div>
    <p role="status">{status}</p>
    <div className="table-wrap"><table><thead><tr><th>Vị trí trong trang</th><th>Hành động</th><th>Resource</th><th>Actor</th><th>Thời điểm</th><th>Digest</th></tr></thead>
      <tbody>{items.map((row,index)=><tr key={row.id}><td>{index+1}</td><td>{row.action}</td><td>{row.resource_id}</td><td>{row.actor_id}</td><td>{row.occurred_at}</td><td><code>{row.digest.slice(0,16)}…</code></td></tr>)}</tbody></table></div>
    <div className="actions"><button disabled={!connected||busy} onClick={()=>void load()}>Trang đầu</button><button disabled={!cursor||busy} onClick={()=>void load(cursor)}>Trang tiếp</button></div>
  </section>;
}
