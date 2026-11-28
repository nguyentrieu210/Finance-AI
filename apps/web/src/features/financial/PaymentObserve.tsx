import {useState,useRef} from 'react';
import {api,errorMessage,isUuid,key,type Material} from './financeClient';
export function PaymentObserve({token,material,onApplied}:{token:string;material:Material;onApplied:()=>void}){
 const [dossier,setDossier]=useState(''),[version,setVersion]=useState(''),[selected,setSelected]=useState(''),[reason,setReason]=useState(''),[status,setStatus]=useState(''),[busy,setBusy]=useState(false);
 const attempt=useRef<{fingerprint:string;key:string}|null>(null);
 const event=material.payment_events?.find(e=>e.id===selected&&!e.observed);
 async function apply(){if(!event||!isUuid(dossier.trim())||!version.trim()||!reason.trim())return;
 const row=material.records.find(r=>r.source_key===event.source_key);if(!row)return;
 const body=JSON.stringify({reference_id:event.id,evidence_ids:row.evidence_ids,reason:reason.trim()});
 const fingerprint=dossier+version+body;if(attempt.current?.fingerprint!==fingerprint)attempt.current={fingerprint,key:key('payment-observe')};
 setBusy(true);try{const r=await api(token,'/v1/dossiers/'+dossier.trim()+'/payment-observations',{method:'POST',headers:{'If-Finance-Version':version.trim(),'Idempotency-Key':attempt.current.key},body});setVersion(r.version??'');setStatus('Đã ghi nhận sự kiện vào sổ phân bổ. Không phát lệnh chuyển tiền.');setSelected('');onApplied()}catch(e){setStatus(errorMessage(e))}finally{setBusy(false)}
 }
 return <section><h3>Ghi nhận giao dịch đã xác minh</h3><label>Giao dịch<select value={selected} onChange={e=>setSelected(e.target.value)}><option value="">Chọn sự kiện</option>{material.payment_events?.map(e=><option key={e.id} value={e.id} disabled={e.observed}>{e.source_key} · {e.event_kind} · {e.amount} {e.currency}{e.observed?' · đã ghi nhận':''}</option>)}</select></label><label>UUID hồ sơ đã chấp nhận<input value={dossier} onChange={e=>setDossier(e.target.value)}/></label><label>Finance-Version<input value={version} onChange={e=>setVersion(e.target.value)}/></label><label>Lý do ghi nhận<input value={reason} onChange={e=>setReason(e.target.value)}/></label><button disabled={busy||!event||!isUuid(dossier.trim())||!version.trim()||!reason.trim()} onClick={()=>void apply()}>Ghi nhận giao dịch</button><p role="status">{status}</p></section>
}
