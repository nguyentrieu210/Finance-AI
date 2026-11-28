import {useEffect,useRef,useState} from 'react';
import {Button} from './Controls';
import type {Member} from './OrganizationAdmin';
const roles={ORG_ADMIN:'Quản trị tổ chức',REQUESTER:'Nhân viên lập hồ sơ',ACCOUNTANT:'Kế toán kiểm soát',AUDITOR:'Kiểm toán',FINANCE_APPROVER:'Người phê duyệt',POLICY_ADMIN:'Quản trị chính sách'};
type Action={kind:'grant';companyId:string}|{kind:'revoke';member:Member};
const uuid=/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
export function MembershipDialog({action,close,onSaved}:{action:Action;close:()=>void;onSaved:()=>void}){
 const modal=useRef<HTMLDialogElement>(null),command=useRef<{key:string;body:string;version?:string}|null>(null);
 const [subject,setSubject]=useState(''),[selected,setSelected]=useState<string[]>([]),[reason,setReason]=useState(''),[error,setError]=useState(''),[busy,setBusy]=useState(false),[uncertain,setUncertain]=useState(false);
 useEffect(()=>{modal.current?.showModal();return()=>modal.current?.close()},[]);
 const valid=action.kind==='grant'?uuid.test(subject.trim())&&selected.length>0:reason.trim().length>0;
 async function save(){
  if(busy||!valid)return;setBusy(true);setError('');
  try{
   const body=action.kind==='grant'?JSON.stringify({company_id:action.companyId,subject_id:subject.trim(),roles:[...selected].sort()}):JSON.stringify({reason:reason.trim()});
   if(!command.current){
    let version:string|undefined;
    if(action.kind==='revoke'){
     const get=await fetch('/v1/memberships/'+encodeURIComponent(action.member.id),{cache:'no-store'});
     if(!get.ok)throw new Error('Không đọc được phiên bản quyền hiện tại ('+get.status+').');
     version=get.headers.get('Finance-Version')??undefined;
     if(!version)throw new Error('API chưa trả mã phiên bản quyền.');
    }
    command.current={key:crypto.randomUUID(),body,version};
   }
   const cmd=command.current;
   if(cmd.body!==body)throw new Error('Nội dung đã thay đổi; đóng và mở lại để thực hiện lệnh mới.');
   const headers:Record<string,string>={'Content-Type':'application/json','Idempotency-Key':cmd.key};
   if(cmd.version)headers['If-Finance-Version']=cmd.version;
   const path=action.kind==='grant'?'/v1/memberships':'/v1/memberships/'+encodeURIComponent(action.member.id)+'/revoke';
   setUncertain(true);
   const r=await fetch(path,{method:'POST',headers,body:cmd.body,cache:'no-store'});
   if(r.ok){onSaved();return}
   if(r.status<500){command.current=null;setUncertain(false)}
   throw new Error(r.status===403?'Không được phép thực hiện thao tác này.':r.status===409?'Quyền đã thay đổi hoặc thành viên đã có quyền. Hãy tải lại dữ liệu.':r.status===401?'Phiên đăng nhập hết hạn.':'Thao tác chưa thành công ('+r.status+').');
  }catch(e){setError((e as Error).message)}finally{setBusy(false)}
 }
 return <dialog ref={modal} className="nv-profile-dialog nv-membership-dialog" onCancel={e=>{e.preventDefault();if(!busy)close()}}>
  <div className="nv-profile-head"><strong>{action.kind==='grant'?'Cấp quyền thành viên':'Thu hồi quyền thành viên'}</strong><Button variant="icon" aria-label="Đóng" disabled={busy} onClick={close}>×</Button></div>
  <form className="nv-profile-body nv-membership-form" onSubmit={e=>{e.preventDefault();void save()}}>
   {action.kind==='grant'?<><label>Mã danh tính nội bộ<input required value={subject} disabled={busy||uncertain} onChange={e=>setSubject(e.target.value)} placeholder="UUID của danh tính đã xác minh"/></label><p>Chỉ cấp quyền cho danh tính đã được khởi tạo trong công ty. Chưa hỗ trợ mời bằng email tại màn này.</p><fieldset disabled={busy||uncertain}><legend>Vai trò</legend>{Object.entries(roles).map(([id,label])=><label key={id} className="nv-role-option"><input type="checkbox" checked={selected.includes(id)} onChange={e=>setSelected(v=>e.target.checked?[...v,id]:v.filter(x=>x!==id))}/>{label}</label>)}</fieldset></>:<><p>Thu hồi toàn bộ vai trò của thành viên này trong công ty.</p><label>Mã thành viên<input readOnly value={action.member.subject_id}/></label><label>Lý do<textarea required maxLength={500} value={reason} disabled={busy||uncertain} onChange={e=>setReason(e.target.value)}/></label></>}
   {error&&<p role="alert" className="nv-org-error">{error}</p>}
   {uncertain&&<p>Chưa xác nhận kết quả. Thử lại sử dụng cùng mã yêu cầu để tránh thao tác trùng.</p>}
   <div className="nv-row"><Button disabled={busy} onClick={close}>Đóng</Button><Button variant="primary" type="submit" disabled={!valid||busy}>{busy?'Đang xử lý…':uncertain?'Thử lại':action.kind==='grant'?'Cấp quyền':'Thu hồi'}</Button></div>
  </form>
 </dialog>
}
