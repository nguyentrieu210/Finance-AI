import {useEffect,useState} from 'react';
import {Button} from './Controls';

type Result={id:string;status:string};
export function InvitationClaim(){
 const [attempt,setAttempt]=useState(0),[state,setState]=useState<'loading'|'pending'|'missing'|'expired'|'error'>('loading');
 useEffect(()=>{
  let active=true;
  setState('loading');
  fetch('/v1/organizations/onboarding/claim',{method:'POST',credentials:'same-origin',cache:'no-store'})
   .then(async response=>{
    if(!active)return;
    if(response.ok){const result=await response.json() as Result;setState(result.status==='PENDING'?'pending':'error');return}
    setState(response.status===403?'missing':response.status===401?'expired':'error');
   })
   .catch(()=>{if(active)setState('error')});
  return()=>{active=false};
 },[attempt]);
 return <section className="nv-card nv-panel nv-org" aria-live="polite">
  <h1>Tham gia tổ chức</h1>
  {state==='loading'?<p role="status">Đang xác minh lời mời…</p>:null}
  {state==='pending'?<><p>Yêu cầu tham gia đã được gửi đến quản trị viên. Tài khoản chỉ được cấp quyền sau khi được phê duyệt.</p><p>Bạn có thể đóng trang này; trạng thái sẽ được cập nhật trong tổ chức sau khi duyệt.</p></>:null}
  {state==='missing'?<p role="alert">Không tìm thấy lời mời còn hiệu lực cho email đã xác minh trong phiên đăng nhập này. Hãy đăng nhập bằng đúng email được mời.</p>:null}
  {state==='expired'?<p role="alert">Phiên đăng nhập đã hết hạn. Hãy đăng nhập lại để tiếp tục nhận lời mời.</p>:null}
  {state==='error'?<p role="alert">Chưa thể xử lý lời mời do dịch vụ không phản hồi. Hãy thử lại sau.</p>:null}
  {(state==='missing'||state==='expired'||state==='error')&&<Button variant="primary" onClick={()=>setAttempt(v=>v+1)}>Thử lại</Button>}
 </section>;
}
