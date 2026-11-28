import {useEffect,useState} from 'react';
import {Button} from './Controls';
import type {components,operations} from '../generated/api';
type Kind='positions'|'departments'|'invitations'|'roles'|'assignments';
type Row=components['schemas']['StaffConfigurationRow'];
type SaveRequest=operations['saveStaffConfiguration']['requestBody']['content']['application/json'];
type Member={id:string;subject_id:string;display_name?:string;email?:string;active:boolean};
type Unit={id:string;name:string;kind:string;active:boolean};
const labels:Record<Kind,string>={invitations:'Lời mời nhân viên',positions:'Chức vụ',departments:'Phân công phòng ban',roles:'Vai trò tùy chỉnh',assignments:'Gán quyền'};
type Capability=NonNullable<SaveRequest['payload']['capabilities']>[number];
const caps:Record<Capability,string>={READ_OWN:'Xem hồ sơ của mình',READ_ASSIGNED:'Xem hồ sơ được giao',READ_DEPARTMENT:'Xem hồ sơ phòng ban',READ_COMPANY:'Xem hồ sơ công ty',WRITE_OWN:'Lập hồ sơ của mình',REVIEW_ASSIGNED:'Kiểm tra hồ sơ được giao',CONFIRM_DEPARTMENT:'Xác nhận phòng ban',APPROVE_ASSIGNED:'Phê duyệt hồ sơ được giao'};
const kinds=Object.keys(labels) as Kind[];
const isCapability=(v:string):v is Capability=>Object.hasOwn(caps,v);
const capabilityLabel=(v:string)=>isCapability(v)?caps[v]:v;
const endpoint='/v1/organizations/staff/';
export async function getRows(kind:Kind):Promise<Row[]>{
 let after='',items:Row[]=[];
 for(let page=0;page<20;page++){
  const r=await fetch(endpoint+kind+'?limit=100'+(after?'&after='+encodeURIComponent(after):''),{cache:'no-store',credentials:'same-origin'});
  if(!r.ok)throw Error('Chưa truy cập được cấu hình nhân sự ('+r.status+').');
  const p=await r.json() as {items:Row[];next_after?:string};items=items.concat(p.items);
  if(!p.next_after)return items;after=p.next_after;
 }
 throw Error('Vượt giới hạn danh sách, cần bộ lọc hoặc phân trang.');
}
export function StaffAdmin({members,units}:{members:Member[];units:Unit[]}){
 const [kind,setKind]=useState<Kind>('invitations'),[data,setData]=useState<Record<Kind,Row[]>>({invitations:[],positions:[],departments:[],roles:[],assignments:[]}),[err,setErr]=useState(''),[busy,setBusy]=useState(false),[ready,setReady]=useState(false);
 const [edit,setEdit]=useState<{id:string;version:number|null;key:string;row:Row}|null>(null);
 async function load(){setReady(false);setErr('');try{const rows=await Promise.all(kinds.map(getRows));setData(Object.fromEntries(kinds.map((k,i)=>[k,rows[i]])) as Record<Kind,Row[]>);setReady(true)}catch(e){setErr((e as Error).message)}}
 useEffect(()=>{void load()},[]);
 const memberName=(id?:string)=>members.find(m=>m.subject_id===id)?.display_name||members.find(m=>m.subject_id===id)?.email||id||'—';
 const unitName=(id?:string|null)=>units.find(u=>u.id===id)?.name||id||'—';
 const positionName=(id?:string|null)=>data.positions.find(p=>p.id===id)?.name||id||'—';
 const roleName=(id?:string)=>data.roles.find(r=>r.id===id)?.name||id||'—';
 function open(row?:Row){const id=row?.id||crypto.randomUUID();setEdit({id,version:row?.version??null,key:crypto.randomUUID(),row:row?{...row}:{id,version:0,active:true,capabilities:[]}})}
 function change(p:Partial<Row>){setEdit(e=>e?{...e,row:{...e.row,...p}}:null)}
 async function save(disable=false){if(!edit)return;setBusy(true);setErr('');try{
  const r=edit.row;let payload:SaveRequest['payload'];
  switch(kind){
   case 'invitations':payload=edit.version===null?{email:r.email?.trim()}:{};break;
   case 'positions':payload={name:r.name?.trim(),active:!disable&&r.active!==false};break;
   case 'roles':payload={name:r.name?.trim(),active:!disable&&r.active!==false,capabilities:r.capabilities?.filter(isCapability)};break;
   case 'departments':payload={actor_id:r.actor_id,department_id:r.department_id,position_id:r.position_id||null,is_primary:!!r.is_primary,active:!disable&&r.active!==false};break;
   case 'assignments':payload=edit.version===null?{actor_id:r.actor_id,role_id:r.role_id,role_version:data.roles.find(v=>v.id===r.role_id)?.version,department_id:r.department_id||null,valid_until:r.valid_until||null}:{};break;
  }
  const response=await fetch(endpoint+kind,{method:'POST',credentials:'same-origin',headers:{'Content-Type':'application/json','Idempotency-Key':edit.key},body:JSON.stringify({id:edit.id,expected_version:edit.version,payload})});
  if(!response.ok)throw Error(response.status===409?'Dữ liệu xung đột, hết hạn hoặc phạm vi không hợp lệ. Hãy làm mới.':'Không lưu được cấu hình ('+response.status+').');
  setEdit(null);await load();
 }catch(e){setErr((e as Error).message)}finally{setBusy(false)}}
 const row=edit?.row;
 const selectedRole=data.roles.find(r=>r.id===row?.role_id);
 const canCreate=ready&&!busy;
 return <section className="nv-card nv-panel nv-org"><div className="nv-org-toolbar"><div className="nv-org-tabs" role="tablist">{kinds.map(k=><Button key={k} variant="tab" role="tab" aria-selected={kind===k} onClick={()=>{setKind(k);setEdit(null)}}>{labels[k]}</Button>)}</div><Button onClick={()=>void load()}>Làm mới</Button><Button variant="primary" disabled={!canCreate} onClick={()=>open()}>＋ Thêm</Button></div>
 <p>Vai trò tùy chỉnh là cấu hình quản trị, chưa cấp quyền nghiệp vụ tài chính. Lời mời mới được lưu nháp, chưa gửi email.</p>
 {err&&<p role="alert" className="nv-org-error">{err}</p>}
 {ready&&<div className="nv-table"><table><thead><tr><th>Đối tượng</th><th>Thông tin</th><th>Trạng thái</th><th>Phiên bản</th><th>Thao tác</th></tr></thead><tbody>{data[kind].map(r=><tr key={r.id}><td>{r.name||r.email||memberName(r.actor_id)}</td><td>{kind==='departments'?unitName(r.department_id)+' · '+positionName(r.position_id)+(r.is_primary?' · Chính':''):kind==='roles'?r.capabilities?.map(c=>capabilityLabel(c)).join(', '):kind==='assignments'?roleName(r.role_id)+' · '+(r.department_id?unitName(r.department_id):'Công ty')+' · phiên bản '+r.role_version:'—'}</td><td>{r.status||(r.active?'Hoạt động':'Ngừng')}</td><td>{r.version}</td><td><Button disabled={r.active===false||r.status==='CANCELLED'} onClick={()=>open(r)}>{kind==='invitations'||kind==='assignments'?'Thu hồi':'Sửa'}</Button></td></tr>)}{!data[kind].length&&<tr><td colSpan={5}>Chưa có dữ liệu.</td></tr>}</tbody></table></div>}
 {edit&&row&&<div className="nv-org-modal" role="dialog" aria-modal="true"><div className="nv-card nv-panel"><div className="nv-row"><strong>{labels[kind]}</strong><Button variant="icon" aria-label="Đóng" onClick={()=>setEdit(null)}>×</Button></div><form onSubmit={e=>{e.preventDefault();void save()}}>
 {(kind==='positions'||kind==='roles')&&<label>Tên<input required maxLength={200} value={row.name||''} onChange={e=>change({name:e.target.value})}/></label>}
 {kind==='invitations'&&<label>Email<input type="email" required disabled={edit.version!==null} value={row.email||''} onChange={e=>change({email:e.target.value})}/></label>}
 {(kind==='departments'||kind==='assignments')&&<label>Nhân viên<select required disabled={edit.version!==null} value={row.actor_id||''} onChange={e=>change({actor_id:e.target.value})}><option value="">Chọn nhân viên</option>{members.filter(m=>m.active||m.subject_id===row.actor_id).map(m=><option value={m.subject_id} key={m.id}>{memberName(m.subject_id)}</option>)}</select></label>}
 {(kind==='departments'||kind==='assignments')&&<label>Phòng ban<select required={kind==='departments'} disabled={edit.version!==null} value={row.department_id||''} onChange={e=>change({department_id:e.target.value||null})}><option value="">{kind==='departments'?'Chọn phòng ban':'Toàn công ty'}</option>{units.filter(u=>u.kind==='DEPARTMENT'&&(u.active||u.id===row.department_id)).map(u=><option key={u.id} value={u.id}>{u.name}</option>)}</select></label>}
 {kind==='departments'&&<><label>Chức vụ<select value={row.position_id||''} onChange={e=>change({position_id:e.target.value||null})}><option value="">Không có</option>{data.positions.filter(p=>p.active||p.id===row.position_id).map(p=><option key={p.id} value={p.id}>{p.name}</option>)}</select></label><label><input type="checkbox" checked={!!row.is_primary} onChange={e=>change({is_primary:e.target.checked})}/> Phòng ban chính</label></>}
 {kind==='roles'&&<fieldset><legend>Quyền nghiệp vụ dự kiến</legend>{Object.entries(caps).map(([c,l])=><label key={c}><input type="checkbox" checked={row.capabilities?.includes(c)||false} onChange={e=>change({capabilities:e.target.checked?[...(row.capabilities||[]),c]:(row.capabilities||[]).filter(v=>v!==c)})}/>{l}</label>)}<p>Sửa vai trò tạo phiên bản mới, không tự mở rộng quyền gán cũ.</p></fieldset>}
 {kind==='assignments'&&<><label>Vai trò<select required disabled={edit.version!==null} value={row.role_id||''} onChange={e=>change({role_id:e.target.value})}><option value="">Chọn vai trò</option>{data.roles.filter(r=>r.active||r.id===row.role_id).map(r=><option key={r.id} value={r.id}>{r.name}</option>)}</select></label><p>{selectedRole?.capabilities?.map(c=>capabilityLabel(c)).join(', ')}</p><label>Hết hạn<input type="datetime-local" disabled={edit.version!==null} onChange={e=>change({valid_until:e.target.value?new Date(e.target.value).toISOString():null})}/></label></>}
 <div className="nv-row"><Button type="button" onClick={()=>setEdit(null)}>Hủy</Button>{edit.version!==null&&['positions','departments','roles'].includes(kind)&&<Button type="button" disabled={busy} onClick={()=>void save(true)}>Ngừng hoạt động</Button>}<Button type="submit" variant="primary" disabled={busy}>{edit.version!==null&&['invitations','assignments'].includes(kind)?'Xác nhận thu hồi':'Lưu'}</Button></div>
 </form></div></div>}
 </section>
}
