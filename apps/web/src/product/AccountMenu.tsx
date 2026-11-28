import {useEffect,useRef,useState} from 'react';
import './account.css';
type Props={email:string;name?:string};
export function AccountMenu({email,name}:Props){
 const [open,setOpen]=useState(false),[avatar,setAvatar]=useState<string|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState('');
 const menu=useRef<HTMLDivElement>(null),input=useRef<HTMLInputElement>(null),profile=useRef<HTMLDialogElement>(null);const [profileOpen,setProfileOpen]=useState(false);
 const [details,setDetails]=useState<{name:string;email:string}|null>(null);const [profileFailed,setProfileFailed]=useState(false);const [action,setAction]=useState<'profile'|'password'|null>(null);const actionDialog=useRef<HTMLDialogElement>(null);
 const refresh=()=>fetch('/profile-api/me',{cache:'no-store'}).then(async r=>{if(!r.ok)throw Error('profile');setDetails(await r.json());setProfileFailed(false)}).catch(()=>setProfileFailed(true));
 useEffect(()=>{void refresh()},[email]);
 const display=details?.name?.trim()||(name&&!name.includes('@')&&name!==email.split('@')[0]?name.trim():'')||(details?'Chưa cập nhật họ tên':profileFailed?'Không tải được họ tên':'Đang tải họ tên…');const actualEmail=details?.email||email;
 const initials=(details?.name||name?display:'NV').split(/[\s._-]+/).filter(Boolean).slice(0,2).map(s=>s[0].toUpperCase()).join('')||'NV';
 useEffect(()=>{let alive=true;let url:string|undefined;fetch('/profile-api/avatar',{cache:'no-store'}).then(async r=>{if(r.ok){url=URL.createObjectURL(await r.blob());if(alive)setAvatar(url);else URL.revokeObjectURL(url)}}).catch(()=>{});return()=>{alive=false;if(url)URL.revokeObjectURL(url)}},[email]);
 useEffect(()=>{const close=(e:PointerEvent)=>{if(!menu.current?.contains(e.target as Node))setOpen(false)};const key=(e:KeyboardEvent)=>{if(e.key==='Escape')setOpen(false)};document.addEventListener('pointerdown',close);document.addEventListener('keydown',key);return()=>{document.removeEventListener('pointerdown',close);document.removeEventListener('keydown',key)}},[]);
 async function upload(file?:File){
  if(!file)return;setError('');
  if(!['image/png','image/jpeg','image/webp'].includes(file.type)||file.size>2*1024*1024){setError('Chọn ảnh PNG, JPG hoặc WebP tối đa 2 MB.');return}
  setBusy(true);
  try{const r=await fetch('/profile-api/avatar',{method:'POST',headers:{'Content-Type':file.type},body:file});if(!r.ok)throw Error();const a=await fetch('/profile-api/avatar',{cache:'no-store'});if(!a.ok)throw Error();const blob=await a.blob();setAvatar(old=>{if(old)URL.revokeObjectURL(old);return URL.createObjectURL(blob)});}
  catch{setError('Không lưu được ảnh. Vui lòng thử lại.')}finally{setBusy(false)}
 }
 useEffect(()=>{const d=profile.current;if(profileOpen&&d&&!d.open)d.showModal();return()=>{if(d?.open)d.close()}},[profileOpen]);
 useEffect(()=>{const d=actionDialog.current;if(action&&d&&!d.open)d.showModal();return()=>{if(d?.open)d.close()}},[action]);
 function beginAction(kind:'profile'|'password'){setOpen(false);setProfileOpen(false);setAction(kind)}
 function actionLoaded(e:React.SyntheticEvent<HTMLIFrameElement>){try{const w=e.currentTarget.contentWindow;if(w?.location.origin===window.location.origin&&w.location.pathname==='/profile-api/actions/complete'&&w.document.body?.dataset.nvActionReturn==='1'){setAction(null);void refresh();window.dispatchEvent(new Event('focus'))}}catch{}}
 function picture(cls:string){return <span className={cls}>{avatar?<img src={avatar} alt=""/>:initials}</span>}
 return <div className="nv-account-menu" ref={menu}>
  <button className="nv-account-trigger" type="button" aria-expanded={open} aria-label="Thông tin người dùng" onClick={()=>{if(!open)void refresh();setOpen(v=>!v)}}>{picture('nv-user-avatar')}<span className="nv-user-name">{display}</span><span aria-hidden="true">⌄</span></button>
  {open&&<div className="nv-account-popover" aria-label="Thông tin người dùng">
    <div className="nv-profile-summary">{picture('nv-user-avatar nv-user-avatar-large')}<div><strong>{display}</strong><small>{actualEmail}</small></div></div>
    <input ref={input} type="file" accept="image/png,image/jpeg,image/webp" hidden onChange={e=>{void upload(e.target.files?.[0]);e.target.value=''}}/>
    <button type="button" disabled={busy} onClick={()=>input.current?.click()}>▧ <span>{busy?'Đang tải ảnh…':'Thay ảnh đại diện'}</span></button>
    <button type="button" onClick={()=>{setOpen(false);setProfileOpen(true)}}>◎ <span>Hồ sơ cá nhân</span></button>
    <button type="button" onClick={()=>beginAction('password')}>⚿ <span>Đổi mật khẩu</span></button>
    {error&&<p role="alert">{error}</p>}
    <div className="nv-account-separator"/>
    <button type="button" className="nv-account-logout" onClick={()=>window.location.assign('/auth/sign_out?rd=%2Flogin')}>↪ <span>Đăng xuất</span></button>
  </div>}
  {profileOpen&&<dialog className="nv-profile-dialog" ref={profile} aria-labelledby="nv-profile-title" onCancel={()=>setProfileOpen(false)}>
    <div className="nv-profile-head"><strong id="nv-profile-title">Hồ sơ cá nhân</strong><button type="button" className="nv-btn-icon" aria-label="Đóng" onClick={()=>setProfileOpen(false)}>×</button></div>
    <div className="nv-profile-body">
      <div className="nv-profile-summary">{picture('nv-user-avatar nv-user-avatar-large')}<div><strong>{display}</strong><small>{actualEmail}</small></div></div>
      <dl><dt>Họ và tên</dt><dd>{display}</dd><dt>Email</dt><dd>{actualEmail}</dd></dl>
      <button type="button" className="nv-profile-action" onClick={()=>beginAction('profile')}>Chỉnh sửa thông tin</button>
      <button type="button" className="nv-profile-action" onClick={()=>beginAction('password')}>Đổi mật khẩu</button>
    </div>
  </dialog>}
  {action&&<dialog className="nv-profile-dialog nv-action-dialog" ref={actionDialog} aria-labelledby="nv-action-title" onCancel={()=>setAction(null)}>
   <div className="nv-profile-head"><strong id="nv-action-title">{action==='password'?'Đổi mật khẩu':'Chỉnh sửa thông tin'}</strong><button type="button" className="nv-btn-icon" aria-label="Đóng" onClick={()=>setAction(null)}>×</button></div>
   <iframe title={action==='password'?'Đổi mật khẩu':'Chỉnh sửa thông tin'} src={action==='password'?'/profile-api/actions/password':'/profile-api/actions/profile'} onLoad={actionLoaded} sandbox="allow-scripts allow-forms allow-same-origin" referrerPolicy="no-referrer"/>
  </dialog>}
 </div>
}
