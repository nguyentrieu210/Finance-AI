import './login.css';

type Props = { dark:boolean; onToggleTheme:()=>void; available:boolean; checking:boolean; returnTo:string; tlsRequired?:boolean };
export function Login({dark,onToggleTheme,available,checking,returnTo,tlsRequired}:Props){
 return <div className="nv-login" data-theme={dark?'dark':'light'}>
  <header className="nv-login-top">
   <span className="nv-login-env">DEV</span>
   <button type="button" className="nv-theme-btn" onClick={onToggleTheme} aria-label={dark?'Chuyển nền sáng':'Chuyển nền tối'} title={dark?'Chuyển nền sáng':'Chuyển nền tối'} aria-pressed={dark}>
    {dark?<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7"><circle cx="12" cy="12" r="4"/><path d="M12 2v3m0 14v3M2 12h3m14 0h3M5 5l2 2m10 10 2 2M19 5l-2 2M7 17l-2 2"/></svg>:<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7"><path d="M20.9 13.3A9 9 0 0 1 10.7 3.1 9 9 0 1 0 20.9 13.3Z"/></svg>}
   </button>
  </header>
  <main className="nv-login-main">
   <div className="nv-login-card" aria-labelledby="nv-login-title">
    <div className="nv-login-brand">
     <img src="/namviet-mark.png?v=3" width="64" height="64" alt="Nam Việt"/>
     <div><strong>Payment &amp;Advanced NV</strong><span>NAM VIỆT</span></div>
    </div>
    <div className="nv-login-body">
     <h1 id="nv-login-title">Đăng nhập</h1>
     <div className="nv-login-provider">
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.65" aria-hidden="true"><rect x="4" y="10" width="16" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3m-4 4v3"/></svg>
      <div><strong>Tài khoản doanh nghiệp</strong><small>Xác thực tập trung (SSO)</small></div>
     </div>
     <button type="button" className="nv-login-submit" disabled={!available||checking} onClick={()=>window.location.assign("/auth/start?rd="+encodeURIComponent(returnTo))} aria-describedby="nv-login-status">Đăng nhập bằng tài khoản Nam Việt <span aria-hidden="true">→</span></button>
     <p id="nv-login-status" role="status" className="nv-login-status"><span aria-hidden="true" /> {checking?"Đang kiểm tra dịch vụ xác thực…":available?"Đăng nhập an toàn qua Keycloak.":tlsRequired?"Cần bật HTTPS toàn tuyến (Cloudflare Full strict) để đăng nhập.":"Không kết nối được dịch vụ xác thực."}</p>

    </div>
   </div>
   <footer className="nv-login-footer">Payment &amp;Advanced NV · Môi trường phát triển</footer>
  </main>
 </div>
}
