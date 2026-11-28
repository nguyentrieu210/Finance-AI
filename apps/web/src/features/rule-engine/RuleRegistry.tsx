import { useEffect, useRef, useState } from 'react';
import {api,errorMessage,isUuid,key} from '../financial/financeClient';
import type { components } from '../../generated/api';
type Rule=components['schemas']['Rule'];
type Page={items:Rule[];next_cursor?:string};
const sample=JSON.stringify({rule_id:"invoice_total_positive",version:1,condition:{op:"gt",args:[{ref:"invoice.total"},{literal:"0",value_type:"money",currency:"VND"}]},severity:"BLOCK",waivable:false,clause_id:"00000000-0000-0000-0000-000000001901"},null,2);
export function RuleRegistry(){
 const[token,setToken]=useState('');const[connected,setConnected]=useState(false);const[items,setItems]=useState<Rule[]>([]);
 const[cursor,setCursor]=useState<string>();const[policy,setPolicy]=useState('00000000-0000-0000-0000-000000001900');
 const[name,setName]=useState('invoice_total_positive');const[definition,setDefinition]=useState(sample);const[status,setStatus]=useState('Chưa kết nối.');
 const[busy,setBusy]=useState(false);const credential=useRef('');const abort=useRef<AbortController|null>(null);const attempt=useRef<{fingerprint:string;key:string}|null>(null);const sending=useRef(false);
 useEffect(()=>()=>{abort.current?.abort();credential.current='';},[]);
 async function load(next?:string){if(!credential.current||busy)return;abort.current?.abort();const c=new AbortController();abort.current=c;setBusy(true);
  try{const q=new URLSearchParams({limit:'20'});if(next)q.set('cursor',next);const r=await fetch('/v1/rules?'+q,{headers:{Authorization:'Bearer '+credential.current},cache:'no-store',credentials:'omit',signal:c.signal});if(!r.ok)throw new Error(String(r.status));const p=await r.json() as Page;setItems(p.items);setCursor(p.next_cursor);setConnected(true);setStatus('Đã tải '+p.items.length+' rule theo quyền hiện tại.');}catch{if(!c.signal.aborted)setStatus('Không tải được rule.');}finally{if(!c.signal.aborted)setBusy(false);}}
 async function create(){if(!credential.current||busy||sending.current||!isUuid(policy)||!name.trim())return;let parsed:unknown;try{parsed=JSON.parse(definition);}catch{setStatus('Definition không phải JSON hợp lệ.');return}
  const body=JSON.stringify({policy_id:policy,name:name.trim(),definition:parsed});if(attempt.current?.fingerprint!==body)attempt.current={fingerprint:body,key:key('rule-draft')};
  sending.current=true;setBusy(true);try{await api(credential.current,'/v1/rules',{method:'POST',headers:{'Idempotency-Key':attempt.current.key},body});attempt.current=null;
   const q=new URLSearchParams({limit:'20'});const {data}=await api<Page>(credential.current,'/v1/rules?'+q);setItems(data.items);setCursor(data.next_cursor);
   setStatus('Đã tạo rule DRAFT; chưa có hiệu lực cho tới khi policy được checker độc lập publish.');
  }catch(e){setStatus(errorMessage(e));}finally{sending.current=false;setBusy(false);}}
 function clear(){if(sending.current)return;abort.current?.abort();attempt.current=null;credential.current='';setToken('');setConnected(false);setItems([]);setCursor(undefined);setBusy(false);setStatus('Đã ngắt kết nối.');}
 return <section aria-labelledby="rule-title"><h2 id="rule-title">Rule registry</h2><p>Rule DRAFT không chạy live. Requester chỉ đọc rule đã PUBLISHED cùng policy.</p>
 <label htmlFor="rule-token">Token sandbox</label><div className="actions"><input id="rule-token" type="password" autoComplete="off" value={token} disabled={busy||connected} onChange={e=>setToken(e.target.value)}/><button className="primary" disabled={busy||connected||!token.trim()} onClick={()=>{credential.current=token.trim();setToken('');void load();}}>Kết nối</button><button disabled={busy} onClick={clear}>Ngắt</button></div>
 <p role="status">{status}</p><div className="table-wrap"><table><thead><tr><th>Tên</th><th>Version</th><th>Severity</th><th>Waivable</th><th>Clause</th></tr></thead><tbody>{items.map(r=><tr key={r.id}><td>{r.name}</td><td>{r.version}</td><td>{r.severity}</td><td>{r.waivable?'Có':'Không'}</td><td>{r.clause_id}</td></tr>)}</tbody></table></div>
 <div className="actions"><button disabled={!connected||busy} onClick={()=>void load()}>Trang đầu</button><button disabled={!cursor||busy} onClick={()=>void load(cursor)}>Trang tiếp</button></div>
 {connected&&<details><summary>Tạo rule DRAFT</summary><label>Policy ID<input value={policy} onChange={e=>setPolicy(e.target.value)}/></label><label>Tên rule<input value={name} onChange={e=>setName(e.target.value)}/></label><label>Definition<textarea rows={14} value={definition} onChange={e=>setDefinition(e.target.value)}/></label><button disabled={busy||!isUuid(policy)||!name.trim()} onClick={()=>void create()}>Tạo DRAFT</button></details>}
 </section>
}
