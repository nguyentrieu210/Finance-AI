import { useEffect, useRef, useState } from 'react';
import {api,errorMessage,isUuid,key} from '../financial/financeClient';
import type { components } from '../../generated/api';
type Rule=components['schemas']['Rule'];
type Simulation=components['schemas']['Simulation'];
type Page={items:Rule[];next_cursor?:string};
const sample=JSON.stringify({rule_id:"invoice_total_positive",version:1,condition:{op:"gt",args:[{ref:"invoice.total"},{literal:"0",value_type:"money",currency:"VND"}]},severity:"BLOCK",waivable:false,clause_id:""},null,2);

export function RuleRegistry(){
 const[token,setToken]=useState(''),[connected,setConnected]=useState(false),[items,setItems]=useState<Rule[]>([]);
 const[cursor,setCursor]=useState<string>(),[policy,setPolicy]=useState('');
 const[name,setName]=useState('invoice_total_positive'),[definition,setDefinition]=useState(sample),[status,setStatus]=useState('Chưa kết nối.');
 const[selected,setSelected]=useState<Rule|null>(null),[version,setVersion]=useState(''),[reason,setReason]=useState(''),[revision,setRevision]=useState('');
 const[corpus,setCorpus]=useState(''),[simulationId,setSimulationId]=useState(''),[simulation,setSimulation]=useState<Simulation|null>(null);
 const[busy,setBusy]=useState(false);
 const credential=useRef(''),inFlight=useRef(false),attempts=useRef(new Map<string,string>());
 useEffect(()=>()=>{credential.current='';attempts.current.clear();},[]);
 function resetSelection(){setSelected(null);setVersion('');setReason('');setRevision('');setSimulation(null);setSimulationId('');}
 function requestKey(path:string,body:string,guard?:string){
  const fingerprint=JSON.stringify([path,guard??null,body]);
  const old=attempts.current.get(fingerprint);if(old)return old;
  // Fail closed rather than evicting an uncertain request's key.
  if(attempts.current.size>=128)throw new Error('Có quá nhiều yêu cầu chưa xác nhận. Kiểm tra kết quả trước khi tiếp tục.');
  const next=key('rule');attempts.current.set(fingerprint,next);return next;
 }
 async function execute(work:()=>Promise<void>){
  if(!credential.current||inFlight.current)return;
  inFlight.current=true;setBusy(true);
  try{await work()}catch(e){setStatus(errorMessage(e))}
  finally{inFlight.current=false;setBusy(false)}
 }
 async function list(next?:string){
  const q=new URLSearchParams({limit:'20'});if(next)q.set('cursor',next);
  const {data}=await api<Page>(credential.current,'/v1/rules?'+q);
  setItems(data.items);setCursor(data.next_cursor);setConnected(true);setStatus('Đã tải '+data.items.length+' rule theo quyền hiện tại.');
 }
 function selectRule(row:Rule,guard:string|null){
  setSelected(row);setVersion(guard??'');setReason('');setSimulation(null);setSimulationId('');
  setRevision(JSON.stringify({...row.definition,version:row.version+1},null,2));
 }
 async function inspect(id:string){
  if(!isUuid(id))return;
  await execute(async()=>{const r=await api<Rule>(credential.current,'/v1/rules/'+id);selectRule(r.data,r.version);setStatus('Đã tải phiên bản '+r.data.version+'.');});
 }
 async function create(){
  if(!isUuid(policy)||!name.trim())return;
  await execute(async()=>{
   const parsed:unknown=JSON.parse(definition);
   const path='/v1/rules',body=JSON.stringify({policy_id:policy,name:name.trim(),definition:parsed});
   const r=await api<Rule>(credential.current,path,{method:'POST',headers:{'Idempotency-Key':requestKey(path,body)},body});
   selectRule(r.data,r.version);setStatus('Đã tạo bản nháp. Chưa có hiệu lực.');
   await list();
  });
 }
 async function revise(){
  if(!selected||!version||!reason.trim())return;
  await execute(async()=>{
   const parsed:unknown=JSON.parse(revision),path='/v1/rules/'+selected.id+'/revise';
   const body=JSON.stringify({name:selected.name,definition:parsed,reason:reason.trim()});
   const r=await api<Rule>(credential.current,path,{method:'POST',headers:{'Idempotency-Key':requestKey(path,body,version),'If-Finance-Version':version},body});
   selectRule(r.data,r.version);await list();setStatus('Đã tạo phiên bản nháp '+r.data.version+'. Chưa công bố chính sách.');
  });
 }
 async function simulate(){
  if(!selected||!version||!isUuid(corpus.trim()))return;
  await execute(async()=>{
   const path='/v1/rules/'+selected.id+'/simulate',body=JSON.stringify({corpus_id:corpus.trim()});
   const r=await api<Simulation>(credential.current,path,{method:'POST',headers:{'Idempotency-Key':requestKey(path,body,version),'If-Finance-Version':version},body});
   setSimulation(r.data);setSimulationId(r.data.id);setStatus('Đã mô phỏng trên bộ dữ liệu được duyệt. Không làm thay đổi kết quả hồ sơ thực tế.');
  });
 }
 async function readSimulation(){
  if(!isUuid(simulationId.trim()))return;
  await execute(async()=>{const {data}=await api<Simulation>(credential.current,'/v1/simulations/'+simulationId.trim());setSimulation(data);setStatus('Đã tải kết quả mô phỏng lưu trữ.');});
 }
 function clear(){if(inFlight.current)return;credential.current='';attempts.current.clear();setToken('');setConnected(false);setItems([]);setCursor(undefined);resetSelection();setCorpus('');setStatus('Đã ngắt kết nối.');}
 return <section aria-labelledby="rule-title"><h2 id="rule-title">Quản lý quy tắc</h2>
 <div className="actions"><label htmlFor="rule-token">Token</label><input id="rule-token" type="password" autoComplete="off" value={token} disabled={busy||connected} onChange={e=>setToken(e.target.value)}/><button className="primary" disabled={busy||connected||!token.trim()} onClick={()=>{credential.current=token.trim();setToken('');void execute(()=>list());}}>Kết nối</button><button disabled={busy} onClick={clear}>Ngắt</button></div>
 <p role="status">{status}</p><div className="table-wrap"><table><thead><tr><th>Tên</th><th>Version</th><th>Severity</th><th>Waivable</th><th>Clause</th><th>Thao tác</th></tr></thead><tbody>{items.map(r=><tr key={r.id}><td>{r.name}</td><td>{r.version}</td><td>{r.severity}</td><td>{r.waivable?'Có':'Không'}</td><td>{r.clause_id}</td><td><button disabled={busy} onClick={()=>void inspect(r.id)}>Chi tiết</button></td></tr>)}</tbody></table></div>
 <div className="actions"><button disabled={!connected||busy} onClick={()=>void execute(()=>list())}>Trang đầu</button><button disabled={!cursor||busy} onClick={()=>void execute(()=>list(cursor))}>Trang tiếp</button></div>
 {connected&&<><details><summary>Tạo rule DRAFT</summary><label>Policy ID<input disabled={busy} value={policy} onChange={e=>setPolicy(e.target.value)}/></label><label>Tên rule<input disabled={busy} value={name} onChange={e=>setName(e.target.value)}/></label><label>Definition<textarea disabled={busy} rows={14} value={definition} onChange={e=>setDefinition(e.target.value)}/></label><button disabled={busy||!isUuid(policy)||!name.trim()} onClick={()=>void create()}>Tạo DRAFT</button></details>
 {selected&&<section><h3>{selected.name} · phiên bản {selected.version}</h3><label>Finance-Version<input readOnly value={version}/></label><details><summary>Tạo phiên bản nháp mới</summary><p>Chỉ chủ sở hữu bản nháp được sửa. Quy tắc đã công bố không thể sửa tại đây.</p><label>Definition<textarea disabled={busy} rows={12} value={revision} onChange={e=>setRevision(e.target.value)}/></label><label>Lý do<input disabled={busy} value={reason} onChange={e=>setReason(e.target.value)}/></label><button disabled={busy||!version||!reason.trim()} onClick={()=>void revise()}>Lưu phiên bản nháp</button></details>
 <label>UUID bộ dữ liệu mô phỏng đã duyệt<input disabled={busy} value={corpus} onChange={e=>setCorpus(e.target.value)}/></label><button disabled={busy||!version||!isUuid(corpus.trim())} onClick={()=>void simulate()}>Mô phỏng rule</button></section>}
 <section><h3>Kết quả mô phỏng</h3><label>UUID kết quả<input disabled={busy} value={simulationId} onChange={e=>{setSimulationId(e.target.value);setSimulation(null);}}/></label><button disabled={busy||!isUuid(simulationId.trim())} onClick={()=>void readSimulation()}>Đọc kết quả</button>
 {simulation&&<table><thead><tr><th>PASS</th><th>FAIL</th><th>UNKNOWN</th><th>ERROR</th><th>NOT_APPLICABLE</th></tr></thead><tbody><tr>{[simulation.pass_count,simulation.fail_count,simulation.unknown_count,simulation.error_count,simulation.not_applicable_count].map((n,i)=><td key={i}>{n??'—'}</td>)}</tr></tbody></table>}</section></>}
 </section>
}
