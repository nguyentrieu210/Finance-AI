import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';
type Metric = components['schemas']['Metric'];

export function Dashboard() {
  const [token,setToken]=useState(''); const [connected,setConnected]=useState(false);
  const [busy,setBusy]=useState(false); const [items,setItems]=useState<Metric[]>([]);
  const [status,setStatus]=useState('Chưa kết nối.'); const credential=useRef('');
  const abort=useRef<AbortController|null>(null);
  useEffect(()=>()=>{abort.current?.abort();credential.current='';},[]);
  async function load(){
    if(!credential.current||busy)return; abort.current?.abort();const controller=new AbortController();abort.current=controller;setBusy(true);setStatus('Đang tải metric hiện trạng...');
    try{const response=await fetch('/v1/dashboard?limit=50',{headers:{Authorization:'Bearer '+credential.current},cache:'no-store',credentials:'omit',signal:controller.signal});
      if(!response.ok){if(response.status===401||response.status===403){credential.current='';setConnected(false);}throw new Error(String(response.status));}
      const page=await response.json() as {items:Metric[]};if(!Array.isArray(page.items))throw new Error('invalid');setItems(page.items);setConnected(true);setStatus('Đã tải metric có thể chứng minh từ trạng thái hiện tại.');
    }catch{if(!controller.signal.aborted){setItems([]);setStatus('Không tải được dashboard.');}}finally{if(!controller.signal.aborted)setBusy(false);}
  }
  function clear(){abort.current?.abort();credential.current='';setToken('');setConnected(false);setBusy(false);setItems([]);setStatus('Đã ngắt kết nối.');}
  return <section aria-labelledby="dashboard-title"><h2 id="dashboard-title">Dashboard vận hành</h2>
    <p>KPI duyệt lần đầu chỉ hiển thị khi hồ sơ đã duyệt có lịch sử quyết định đầy đủ; thời gian xử lý chưa được suy diễn.</p>
    <label htmlFor="dashboard-token">Token Requester sandbox</label><div className="actions"><input id="dashboard-token" type="password" autoComplete="off" maxLength={16384} value={token} disabled={busy||connected} onChange={e=>setToken(e.target.value)}/>
      <button className="primary" disabled={busy||connected||!token.trim()} onClick={()=>{credential.current=token.trim();setToken('');void load();}}>Kết nối</button><button disabled={busy} onClick={clear}>Ngắt kết nối</button></div>
    <p role="status">{status}</p><div className="table-wrap"><table><thead><tr><th>Metric</th><th>Giá trị</th><th>Đơn vị</th><th>Mẫu số</th><th>As of</th></tr></thead>
      <tbody>{items.map(row=><tr key={row.name}><td>{row.name}</td><td>{row.value}</td><td>{row.unit}</td><td>{row.denominator ?? '—'}</td><td>{row.as_of}</td></tr>)}</tbody></table></div>
    <div className="actions"><button disabled={!connected||busy} onClick={()=>void load()}>Làm mới</button></div>
  </section>;
}
