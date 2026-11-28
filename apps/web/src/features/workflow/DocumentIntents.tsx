import { useEffect, useRef, useState } from 'react';
import type { components } from '../../generated/api';

type Document = components['schemas']['Document'];
type Intent = components['schemas']['UploadIntent'];
type Dossier = components['schemas']['Dossier'];
type Metadata = { file_name:string; mime:string; bytes:number; sha256:string; kind:string };
type Pending = { key:string; version:string; body:Metadata };

const allowed = new Set(['application/pdf','image/jpeg','image/png']);
const maxBytes = 25*1024*1024;
const kinds = ['INVOICE','CONTRACT','PAYMENT_REQUEST','ADVANCE_REQUEST','SETTLEMENT','ACCEPTANCE_RECORD','OTHER'];

export function DocumentIntents({dossier,token}:{dossier:Dossier;token:string}) {
  const [items,setItems]=useState<Document[]>([]);
  const [cursor,setCursor]=useState<string>();
  const [status,setStatus]=useState('Chưa tải danh sách chứng từ.');
  const [busy,setBusy]=useState(false);
  const [uncertain,setUncertain]=useState(false);
  const [denied,setDenied]=useState(false);
  const [kind,setKind]=useState('INVOICE');
  const [metadata,setMetadata]=useState<Omit<Metadata,'kind'>>();
  const [version,setVersion]=useState(dossier.version);
  const pending=useRef<Pending|null>(null);
  const controller=useRef<AbortController|null>(null);
  const active=useRef(false);
  const generation=useRef(0);
  useEffect(()=>()=>{generation.current++;controller.current?.abort();active.current=false;pending.current=null;},[]);
  const path='/v1/dossiers/'+encodeURIComponent(dossier.id);
  function start(){
    if(active.current||!token||denied)return null;
    active.current=true;setBusy(true);
    const abort=new AbortController();controller.current=abort;
    return {abort,serial:++generation.current};
  }
  function finish(serial:number){
    if(serial===generation.current){active.current=false;setBusy(false);}
  }
  function reject(statusCode:number){
    if(statusCode===401||statusCode===403){
      setDenied(true);setItems([]);setCursor(undefined);setMetadata(undefined);pending.current=null;setUncertain(false);
    }
    setStatus('Không thể thực hiện ('+statusCode+').');
  }
  function fetchData(url:string,abort:AbortController,options:RequestInit={}){
    return fetch(url,{...options,headers:{...options.headers,Authorization:'Bearer '+token},cache:'no-store',credentials:'omit',signal:abort.signal});
  }
  async function refresh(next?:string){
    const op=start();if(!op)return;
    try{
      const draftResponse=await fetchData(path,op.abort);
      if(op.serial!==generation.current)return;
      if(!draftResponse.ok){reject(draftResponse.status);return;}
      const current=await draftResponse.json() as Dossier;
      const tag=draftResponse.headers.get('finance-version');
      if(current.id!==dossier.id||current.status!=='DRAFT'||tag!==('"v1:dossier:'+dossier.id+':'+current.version+'"'))throw Error('Invalid version');
      const params=new URLSearchParams({limit:'10'});if(next)params.set('cursor',next);
      const response=await fetchData(path+'/documents?'+params,op.abort);
      if(op.serial!==generation.current)return;
      if(!response.ok){reject(response.status);return;}
      const data=await response.json() as {items:Document[];next_cursor?:string};
      if(op.serial!==generation.current)return;
      if(!Array.isArray(data.items))throw Error('Invalid list');
      setItems(data.items);setCursor(data.next_cursor);setVersion(current.version);
      setStatus('Đã tải '+data.items.length+' chứng từ metadata.');
    }catch{if(op.serial===generation.current&&!op.abort.signal.aborted)setStatus('Không tải được metadata.');}
    finally{finish(op.serial);}
  }
  async function choose(file:File|undefined){
    if(active.current||pending.current||denied)return;
    setMetadata(undefined);
    if(!file)return;
    if(!allowed.has(file.type)||file.size<1||file.size>maxBytes){setStatus('Chỉ PDF/JPEG/PNG tổng hợp, tối đa 25 MiB.');return;}
    const op=start();if(!op)return;
    try{
      const digest=await crypto.subtle.digest('SHA-256',await file.arrayBuffer());
      if(op.serial!==generation.current)return;
      const sha256=Array.from(new Uint8Array(digest),b=>b.toString(16).padStart(2,'0')).join('');
      setMetadata({file_name:file.name,mime:file.type,bytes:file.size,sha256});
      setStatus('Đã tính SHA-256 tại máy. Chưa gửi nội dung file.');
    }catch{if(op.serial===generation.current)setStatus('Không tính được SHA-256.');}
    finally{finish(op.serial);}
  }
  async function reserve(){
    if(!metadata&&!pending.current)return;
    const op=start();if(!op)return;
    try{
      if(!pending.current){
        if(!metadata)throw Error('Missing metadata');
        pending.current={key:crypto.randomUUID(),version:'"v1:dossier:'+dossier.id+':'+version+'"',body:{...metadata,kind}};
      }
      const cmd=pending.current;
      const response=await fetchData(path+'/documents',op.abort,{
        method:'POST',headers:{'Content-Type':'application/json','Idempotency-Key':cmd.key,'If-Finance-Version':cmd.version},
        body:JSON.stringify(cmd.body),
      });
      if(op.serial!==generation.current)return;
      if(!response.ok){
        if(response.status>=500){setUncertain(true);setStatus('Chưa rõ kết quả. Thử lại cùng yêu cầu.');}
        else{pending.current=null;setUncertain(false);reject(response.status);}
        return;
      }
      const receipt=await response.json() as Intent;
      if(op.serial!==generation.current)return;
      if(!receipt.document_id||receipt.state!=='PENDING_UPLOAD'||receipt.required_sha256!==cmd.body.sha256)throw Error('Invalid receipt');
      pending.current=null;setUncertain(false);setMetadata(undefined);
      finish(op.serial);await refresh();
    }catch{if(op.serial===generation.current&&!op.abort.signal.aborted){setUncertain(true);setStatus('Chưa rõ kết quả. Thử lại cùng yêu cầu.');}}
    finally{finish(op.serial);}
  }
  return <div className="draft-detail" role="region" aria-label="Metadata chứng từ">
    <h3>Metadata chứng từ</h3>
    <p>Chỉ chọn file tổng hợp. Hệ thống chỉ gửi tên, loại, kích thước và mã SHA-256; chưa tải nội dung file lên. Trạng thái chờ upload không phải chứng từ đã xác minh.</p>
    <p>Phiên bản hồ sơ: {version}</p>
    <div className="actions"><button disabled={busy||denied||uncertain} onClick={()=>void refresh()}>Tải metadata</button><button disabled={busy||denied||uncertain||!cursor} onClick={()=>void refresh(cursor)}>Trang chứng từ tiếp</button></div>
    <div className="draft-controls">
      <div><label htmlFor="synthetic-document">File tổng hợp</label><input id="synthetic-document" type="file" accept=".pdf,.png,.jpg,.jpeg" disabled={busy||denied||uncertain} onChange={e=>void choose(e.target.files?.[0])}/></div>
      <div><label htmlFor="document-kind">Loại chứng từ</label><select id="document-kind" value={kind} disabled={busy||denied||uncertain} onChange={e=>setKind(e.target.value)}>{kinds.map(k=><option key={k}>{k}</option>)}</select></div>
      <button className="primary" disabled={busy||denied||(!metadata&&!uncertain)} onClick={()=>void reserve()}>{uncertain?'Thử lại metadata':'Tạo metadata chứng từ'}</button>
    </div>
    <p aria-live="polite" className="document-status">{status}</p>
    {metadata&&<p>{metadata.file_name} · {metadata.bytes} byte · SHA-256: <code>{metadata.sha256}</code></p>}
    <div className="table-wrap"><table><thead><tr><th>Tên file</th><th>Loại</th><th>Trạng thái</th></tr></thead><tbody>{items.map(row=><tr key={row.id}><td>{row.file_name}</td><td>{row.kind}</td><td>{row.state}</td></tr>)}</tbody></table></div>
  </div>;
}
