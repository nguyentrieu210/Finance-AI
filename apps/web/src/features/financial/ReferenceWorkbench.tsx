import { useEffect, useRef, useState } from 'react';
import { PaymentImport } from './PaymentImport';
import { PaymentObserve } from './PaymentObserve';
import { VerifiedContracts } from '../contracts/VerifiedContracts';
import { VerifiedAdvances } from '../advances/VerifiedAdvances';
import { VerifiedBudgets } from '../budget/VerifiedBudgets';
import { api,errorMessage,ids,isAmount,isUuid,key,kinds,type Row,type Material,type ResourceKind } from './financeClient';
type DraftRow={source_key:string;currency:string;cap:string;consumed:string;evidence:string;details:string};
const empty=():DraftRow=>({source_key:'',currency:'',cap:'',consumed:'',evidence:'',details:'{}'});
const defaultKind:Record<'E12'|'E13'|'E14',ResourceKind>={E12:'CONTRACT',E13:'ADVANCE',E14:'BUDGET'};
export function ReferenceWorkbench({module}:{module:'E12'|'E13'|'E14'}){
 const [token,setToken]=useState(''),[activeToken,setActiveToken]=useState('');
 const [documentId,setDocumentId]=useState(''),[kind,setKind]=useState<ResourceKind>(defaultKind[module]);
 const [coverage,setCoverage]=useState('UNKNOWN'),[asOf,setAsOf]=useState(''),[fresh,setFresh]=useState('');
 const [rows,setRows]=useState<DraftRow[]>([empty()]),[refId,setRefId]=useState(''),[material,setMaterial]=useState<Material|null>(null);
 const [reason,setReason]=useState(''),[evidence,setEvidence]=useState(''),[checked,setChecked]=useState(false),[version,setVersion]=useState<string|null>(null);
 const [busy,setBusy]=useState(false),[status,setStatus]=useState('Chưa kết nối.');
 const credential=useRef(''),attempt=useRef<{body:string;key:string}|null>(null);
 useEffect(()=>{setKind(defaultKind[module]);setRows([empty()]);setMaterial(null);setChecked(false);setVersion(null)},[module]);
 useEffect(()=>()=>{credential.current=''},[]);
 function clear(){credential.current='';setToken('');setActiveToken('');setMaterial(null);setChecked(false);setVersion(null);setStatus('Đã ngắt kết nối.');attempt.current=null;}
 function update(i:number,patch:Partial<DraftRow>){setRows(old=>old.map((r,j)=>i===j?{...r,...patch}:r))}
 async function create(){
  if(!credential.current||!isUuid(documentId.trim())){setStatus('Cần UUID chứng từ nguồn đã quét sạch.');return}
  const dates=[new Date(asOf),new Date(fresh)];
  if(!asOf||!fresh||!dates.every(d=>Number.isFinite(d.getTime()))||dates[1]<=dates[0]||dates[0]>new Date()){setStatus('Kiểm tra thời điểm dữ liệu và hạn hiệu lực.');return}
  let records:Row[];try{records=rows.map(r=>({source_key:r.source_key.trim(),currency:r.currency.trim().toUpperCase(),cap:r.cap.trim(),consumed:r.consumed.trim(),evidence_ids:ids(r.evidence),details:JSON.parse(r.details) as Record<string,unknown>}))}catch{setStatus('Chi tiết nguồn phải là JSON hợp lệ.');return}
  if(!records.length||records.some(r=>!r.source_key||!/^([A-Z]{3})$/.test(r.currency)||!isAmount(r.cap)||!isAmount(r.consumed)||!r.evidence_ids.length||r.evidence_ids.some(x=>!isUuid(x)))||new Set(records.map(r=>r.source_key)).size!==records.length||new Set(records.flatMap(r=>r.evidence_ids)).size>50){setStatus('Mỗi dòng cần mã nguồn duy nhất, tiền tệ, số tiền chính xác và bằng chứng UUID; tối đa 50 bằng chứng mỗi bản nhập.');return}
  const body=JSON.stringify({document_id:documentId.trim(),kind,coverage,as_of:dates[0].toISOString(),fresh_until:dates[1].toISOString(),records});
  if(attempt.current?.body!==body)attempt.current={body,key:key('ref-create')};
  setBusy(true);
  try{const {data}=await api<{id:string}>(credential.current,'/v1/references',{method:'POST',headers:{'Idempotency-Key':attempt.current.key},body});setRefId(data.id);setMaterial(null);setVersion(null);setChecked(false);setStatus('Đã tạo bản nhập '+data.id+'. Chuyển cho người duyệt độc lập.');}
  catch(e){setStatus('Không tạo được: '+errorMessage(e))}finally{setBusy(false)}
 }
 async function inspect(){
  if(!isUuid(refId.trim()))return;
  setBusy(true);setMaterial(null);setChecked(false);setVersion(null);
  try{const path='/v1/references/'+encodeURIComponent(refId.trim());const {data}=await api<Material>(credential.current,path+'/material');const meta=await api<unknown>(credential.current,path);setMaterial(data);setVersion(meta.version);setEvidence(data.evidence_ids.join(', '));setStatus('Đã đọc bản nhập bất biến. Đối chiếu từng dòng với tài liệu nguồn trước khi xác minh.')}
  catch(e){setStatus('Không đọc được bản nhập: '+errorMessage(e))}finally{setBusy(false)}
 }
 async function verify(){
  if(!material||!version||!checked||!reason.trim()||material.id!==refId.trim()||material.state!=='DRAFT')return;
  const ev=ids(evidence);
  if(!ev.length||ev.length>50||ev.some(x=>!isUuid(x))||material.records.some(r=>!r.evidence_ids.some(x=>ev.includes(x)))){setStatus('Phải đối chiếu bằng chứng cho mọi dòng.');return}
  const body=JSON.stringify({reason:reason.trim(),evidence_ids:ev});
  if(attempt.current?.body!==body)attempt.current={body,key:key('ref-verify')};
  setBusy(true);
  try{await api(credential.current,'/v1/references/'+material.id+'/verify',{method:'POST',headers:{'If-Finance-Version':version,'Idempotency-Key':attempt.current.key},body});setMaterial((await api<Material>(credential.current,'/v1/references/'+material.id+'/material')).data);setChecked(false);setStatus('Đã xác minh độc lập. Nguồn đủ điều kiện xuất hiện trong danh sách giữ hạn mức.')}
  catch(e){setStatus('Xác minh thất bại: '+errorMessage(e))}finally{setBusy(false)}
 }
 return <section><h2>Quản trị nguồn tài chính</h2><div className="actions"><input aria-label="Token" type="password" autoComplete="off" value={token} onChange={e=>setToken(e.target.value)} disabled={busy||Boolean(activeToken)}/><button disabled={busy||!token.trim()||Boolean(activeToken)} onClick={()=>{credential.current=token.trim();setActiveToken(credential.current);setToken('');setStatus('Đã kết nối.')}}>Kết nối</button><button disabled={busy} onClick={clear}>Ngắt</button></div>
 {activeToken&&<>
 {module==='E12'&&<VerifiedContracts token={activeToken}/>}
 {module==='E13'&&<VerifiedAdvances token={activeToken}/>}
 {module==='E14'&&<VerifiedBudgets token={activeToken}/>}
 <h3>Nhập bảng hạn mức từ chứng từ nguồn</h3><p>Chi tiết CONTRACT: contract_key, installment_key, vendor_key, effective_from, effective_until, base_cap, addenda (key/approved_on/effective_on/delta), installment_gross, retention, advance_deduction, paid, acceptance_lines (key/item/unit/quantity/unit_price/value, cùng danh mục nghiệm thu trên mọi kỳ). ADVANCE: beneficiary_key, purpose, issued_on, due_date, issued, settled, refunded, extension. BUDGET: cost_center, account, budget_version, period_start, period_end, approved_limit, external_unmatched_commitments, actual, disjoint_verified. INVOICE: issuer, series, number, invoice_date, eligible_total, paid. Số tiền nhập chuỗi thập phân, ngày YYYY-MM-DD; không suy diễn giá trị thiếu.</p><div className="actions"><label>UUID chứng từ<input value={documentId} onChange={e=>setDocumentId(e.target.value)}/></label><label>Loại<select value={kind} onChange={e=>setKind(e.target.value as ResourceKind)}>{kinds.map(k=><option key={k}>{k}</option>)}</select></label><label>Độ phủ<select value={coverage} onChange={e=>setCoverage(e.target.value)}>{['UNKNOWN','PARTIAL','COMPLETE'].map(k=><option key={k}>{k}</option>)}</select></label><label>Thời điểm số liệu<input type="datetime-local" value={asOf} onChange={e=>setAsOf(e.target.value)}/></label><label>Hết hiệu lực<input type="datetime-local" value={fresh} onChange={e=>setFresh(e.target.value)}/></label></div>
 <div className="table-wrap"><table><thead><tr><th>Mã nguồn</th><th>Tiền tệ</th><th>Hạn mức</th><th>Đã sử dụng</th><th>UUID bằng chứng</th><th>Chi tiết nguồn (JSON)</th><th></th></tr></thead><tbody>{rows.map((r,i)=><tr key={i}>{(['source_key','currency','cap','consumed','evidence','details'] as const).map(k=><td key={k}><input aria-label={k+' '+(i+1)} value={r[k]} onChange={e=>update(i,{[k]:e.target.value})}/></td>)}<td><button disabled={busy||rows.length===1} onClick={()=>setRows(old=>old.filter((_,j)=>i!==j))}>Xóa dòng</button></td></tr>)}</tbody></table></div>
 <div className="actions"><button disabled={busy||rows.length>=1000} onClick={()=>setRows(old=>[...old,empty()])}>Thêm dòng</button><button className="primary" disabled={busy} onClick={()=>void create()}>Lưu bản nhập</button></div>
 <PaymentImport token={activeToken}/>
 <h3>Duyệt bản nhập độc lập</h3><div className="actions"><input aria-label="UUID bản nhập" value={refId} onChange={e=>{setRefId(e.target.value);setMaterial(null);setChecked(false);setVersion(null)}}/><button disabled={busy} onClick={()=>void inspect()}>Đọc bản nhập</button></div>
 {material&&<><p>{material.kind} · {material.coverage} · {material.state} · Hết hạn {material.fresh_until}</p><p>Nguồn {material.document_id} · SHA-256 <code>{material.source_digest}</code></p><div className="table-wrap"><table><thead><tr><th>Mã nguồn</th><th>Tiền tệ</th><th>Hạn mức</th><th>Đã dùng</th><th>Bằng chứng</th><th>Chi tiết</th></tr></thead><tbody>{material.records.map(r=><tr key={r.source_key}><td>{r.source_key}</td><td>{r.currency}</td><td>{'cap' in r?r.cap:r.amount}</td><td>{'consumed' in r?r.consumed:r.event_kind+' · '+r.reservation_id+(r.reverses?' · đảo '+r.reverses:'')}</td><td>{r.evidence_ids.join(', ')}</td><td>{'details' in r?<pre>{JSON.stringify(r.details,null,2)}</pre>:r.occurred_at}</td></tr>)}</tbody></table></div><label>UUID bằng chứng đã đối chiếu<textarea value={evidence} onChange={e=>{setEvidence(e.target.value);setChecked(false)}}/></label><label>Lý do xác minh<input value={reason} onChange={e=>setReason(e.target.value)}/></label><label><input type="checkbox" checked={checked} onChange={e=>setChecked(e.target.checked)}/> Đã đối chiếu mọi dòng với nguồn, xác nhận độ phủ và hiệu lực.</label><button className="primary" disabled={busy||!checked||!version||material.coverage!=='COMPLETE'||material.state!=='DRAFT'||!reason.trim()} onClick={()=>void verify()}>Xác minh</button></>}
 {material?.kind==='PAYMENT'&&material.state==='APPROVED'&&<PaymentObserve token={activeToken} material={material} onApplied={()=>void inspect()}/>}
 </>}<p role="status">{status}</p></section>
}
