import {useRef,useState} from 'react';
import {LineReconciliation} from './LineReconciliation';
import {api,errorMessage,isUuid,key} from '../financial/financeClient';
import type {components} from '../../generated/api';
type Match=components['schemas']['Match'];

export function MatchingWorkbench(){
 const [token,setToken]=useState(''),[dossier,setDossier]=useState(''),[version,setVersion]=useState(''),[left,setLeft]=useState(''),[right,setRight]=useState(''),[reason,setReason]=useState('');
 const [items,setItems]=useState<Match[]>([]),[status,setStatus]=useState('Chưa kết nối.'),[busy,setBusy]=useState(false),[connected,setConnected]=useState(false);
 const credential=useRef(''),attempt=useRef<{fingerprint:string;key:string}|null>(null),inFlight=useRef(false);
 async function load(){
  if(!credential.current||!isUuid(dossier)||inFlight.current)return;
  inFlight.current=true;setBusy(true);
  try{
   const p=await api<{items:Match[]}>(credential.current,'/v1/dossiers/'+dossier+'/matches?limit=50');
   const d=await api<unknown>(credential.current,'/v1/dossiers/'+dossier);
   setItems(p.data.items);setVersion(d.version??'');setStatus('Đã tải '+p.data.items.length+' match.');
  }catch(e){setItems([]);setVersion('');setStatus(errorMessage(e));}
  finally{inFlight.current=false;setBusy(false);}
 }
 async function confirm(){
  if(!credential.current||!isUuid(dossier)||!isUuid(left)||!isUuid(right)||left.toLowerCase()===right.toLowerCase()||!version||!reason.trim()||inFlight.current)return;
  inFlight.current=true;setBusy(true);
  try{
   const body=JSON.stringify({left_evidence_id:left,right_evidence_id:right,reason:reason.trim()});
   const fingerprint=JSON.stringify([dossier,version,body]);
   if(attempt.current?.fingerprint!==fingerprint)attempt.current={fingerprint,key:key('ui-match')};
   const r=await api<Match>(credential.current,'/v1/dossiers/'+dossier+'/matches',{method:'POST',headers:{'Idempotency-Key':attempt.current.key,'If-Finance-Version':version},body});
   setVersion(r.version??'');setLeft('');setRight('');setReason('');attempt.current=null;
   setStatus('Đã xác nhận cặp bằng chứng. Không phải phê duyệt tài chính.');
   const p=await api<{items:Match[]}>(credential.current,'/v1/dossiers/'+dossier+'/matches?limit=50');
   setItems(p.data.items);
  }catch(e){setStatus(errorMessage(e));}
  finally{inFlight.current=false;setBusy(false);}
 }
 function connect(){
  if(!token.trim()||busy)return;
  credential.current=token.trim();setToken('');setConnected(true);attempt.current=null;setVersion('');setItems([]);void load();
 }
 function disconnect(){
  credential.current='';attempt.current=null;setConnected(false);setItems([]);setVersion('');setToken('');setStatus('Đã ngắt kết nối.');
 }
 return <section><h2>Đối chiếu bằng chứng</h2><p>Fuzzy chỉ là gợi ý. Xác nhận thủ công yêu cầu kế toán được phân công và hai bằng chứng cùng revision.</p>
 <label>Token Accountant<input type="password" autoComplete="off" disabled={busy||connected} value={token} onChange={e=>setToken(e.target.value)}/></label>
 <label>Dossier<input disabled={busy} value={dossier} onChange={e=>{setDossier(e.target.value);setVersion('');setItems([]);attempt.current=null;}}/></label>
 <label>Finance-Version<input readOnly value={version}/></label>
 <label>Left evidence<input disabled={busy} value={left} onChange={e=>setLeft(e.target.value)}/></label>
 <label>Right evidence<input disabled={busy} value={right} onChange={e=>setRight(e.target.value)}/></label>
 <label>Lý do xác nhận<input disabled={busy} value={reason} onChange={e=>setReason(e.target.value)}/></label>
 <div className="actions"><button disabled={busy||connected||!token.trim()} onClick={connect}>Kết nối</button><button disabled={busy||!connected} onClick={disconnect}>Ngắt</button><button disabled={busy||!connected||!isUuid(dossier)} onClick={()=>void load()}>Tải hồ sơ</button><button disabled={busy||!connected||!version||!isUuid(left)||!isUuid(right)||left.toLowerCase()===right.toLowerCase()||!reason.trim()} onClick={()=>void confirm()}>Xác nhận cặp</button></div>
 <p role="status">{status}</p><div className="table-wrap"><table><thead><tr><th>State</th><th>Left</th><th>Right</th><th>Reason</th></tr></thead><tbody>{items.map(x=><tr key={x.id}><td>{x.state}</td><td>{x.left_evidence_id??''}</td><td>{x.right_evidence_id??''}</td><td>{x.reason}</td></tr>)}</tbody></table></div>
 {connected&&isUuid(dossier)&&<LineReconciliation key={dossier} token={credential.current} dossier={dossier}/>}</section>
}
