import hashlib, io, json, os, tempfile, urllib.request, urllib.error, http.client, urllib.parse, logging
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from PIL import Image, ImageOps
Image.MAX_IMAGE_PIXELS=12000000
ROOT=Path(os.environ.get("NV_PROFILE_DIR","/var/lib/namviet-profile"))
LIMIT=2*1024*1024
ORIGIN="https://namviet.gpcds.site"
ACTIONS={"/profile-api/actions/profile":"UPDATE_PROFILE","/profile-api/actions/password":"UPDATE_PASSWORD"}
def action_redirect(location,action):
 u=urllib.parse.urlsplit(location)
 if (u.scheme,u.netloc,u.path)!=("https","namviet.gpcds.site","/realms/namviet/protocol/openid-connect/auth"):
  raise ValueError("Unexpected authorization endpoint")
 q=urllib.parse.parse_qsl(u.query,keep_blank_values=True)
 params=dict(q)
 if params.get("client_id")!="finance-web" or params.get("redirect_uri")!=ORIGIN+"/auth/callback" or params.get("code_challenge_method")!="S256" or not params.get("state") or not params.get("code_challenge"):
  raise ValueError("Invalid authorization request")
 if action not in ACTIONS.values():raise ValueError("Unknown action")
 q=[(k,v) for k,v in q if k not in ("kc_action","prompt")]
 q.append(("kc_action",action))
 if action=="UPDATE_PASSWORD":q.append(("prompt","login"))
 return urllib.parse.urlunsplit((u.scheme,u.netloc,u.path,urllib.parse.urlencode(q),""))
class Handler(BaseHTTPRequestHandler):
 def log_message(self,*args): pass
 def send(self,status,body=b"",kind="application/json"):
  self.send_response(status);self.send_header("Content-Type",kind);self.send_header("Cache-Control","no-store");self.send_header("X-Content-Type-Options","nosniff");self.send_header("Content-Length",str(len(body)));
  for cookie in getattr(self,"session_cookies",[]):self.send_header("Set-Cookie",cookie)
  self.end_headers();self.wfile.write(body)
 def err(self,status,message): self.send(status,json.dumps({"error":message}).encode())
 def identity(self):
  try:
   req=urllib.request.Request("http://127.0.0.1:18191/auth/userinfo",headers={"Cookie":self.headers.get("Cookie",""),"Host":"namviet.gpcds.site","X-Forwarded-Proto":"https"})
   with urllib.request.urlopen(req,timeout=5) as res: d=json.load(res)
   user=d.get("user")
   if not isinstance(user,str) or not user or len(user)>512: return None
   return hashlib.sha256(("namviet:"+user).encode()).hexdigest()
  except (OSError,ValueError): return None
 def profile_info(self):
  c=http.client.HTTPConnection("127.0.0.1",18191,timeout=5)
  kc=None
  try:
   c.request("GET","/auth/auth",headers={"Cookie":self.headers.get("Cookie",""),"Host":"namviet.gpcds.site","X-Forwarded-Host":"namviet.gpcds.site","X-Forwarded-Proto":"https","X-Forwarded-Port":"443","X-Original-URI":"/profile-api/me"})
   r=c.getresponse();token=r.getheader("X-Auth-Request-Access-Token")
   self.session_cookies=[v for k,v in r.getheaders() if k.lower()=="set-cookie"]
   if r.status!=202 or not token:
    logging.warning("Profile auth response status=%s access_token_present=%s",r.status,bool(token))
    return None
   # Trusted loopback Keycloak, no external Cloudflare hop and no token in browser.
   kc=http.client.HTTPConnection("127.0.0.1",18080,timeout=5)
   kc.request("GET","/realms/namviet/protocol/openid-connect/userinfo",headers={"Host":"namviet.gpcds.site","X-Forwarded-Host":"namviet.gpcds.site","X-Forwarded-Proto":"https","X-Forwarded-Port":"443","Authorization":"Bearer "+token,"Accept":"application/json"})
   res=kc.getresponse()
   if res.status!=200:
    logging.warning("Profile issuer userinfo status=%s",res.status)
    return None
   d=json.load(res)
   name=" ".join(x.strip() for x in (d.get("family_name",""),d.get("given_name","")) if isinstance(x,str) and x.strip())
   if not name:
    n=d.get("name","");name=n.strip() if isinstance(n,str) and "@" not in n else ""
   email=d.get("email","")
   if not name:logging.warning("Profile issuer response missing name claims")
   return {"name":name,"email":email if isinstance(email,str) else ""}
  except (OSError,ValueError,http.client.HTTPException) as exc:
   logging.warning("Profile lookup failed: %s",type(exc).__name__)
   return None
  finally:
   c.close()
   if kc:kc.close()
 def start_action(self,action):
  c=http.client.HTTPConnection("127.0.0.1",18191,timeout=8)
  try:
   c.request("GET","/auth/start?rd=%2Fprofile-api%2Factions%2Fcomplete",headers={"Host":"namviet.gpcds.site","X-Forwarded-Host":"namviet.gpcds.site","X-Forwarded-Proto":"https","X-Forwarded-Port":"443","Cookie":self.headers.get("Cookie","")})
   r=c.getresponse()
   if r.status not in (302,303):raise ValueError("Authentication start failed")
   destination=action_redirect(r.getheader("Location",""),action)
   cookies=[v for k,v in r.getheaders() if k.lower()=="set-cookie"]
   if not cookies:raise ValueError("Missing authentication state")
  except (OSError,ValueError,http.client.HTTPException):
   return self.err(503,"Không thể mở phiên xác thực mới. Vui lòng thử lại.")
  finally:c.close()
  self.send_response(303);self.send_header("Location",destination);self.send_header("Cache-Control","no-store");self.send_header("Referrer-Policy","no-referrer")
  for cookie in cookies:self.send_header("Set-Cookie",cookie)
  self.send_header("Content-Length","0");self.end_headers()
 def do_GET(self):
  if self.path=="/profile-api/me":
   d=self.profile_info()
   if d is None:return self.err(503,"Không tải được hồ sơ")
   return self.send(200,json.dumps(d,ensure_ascii=False).encode())
  key=self.identity()
  if not key:return self.err(401,"Unauthenticated")
  if self.path in ACTIONS:return self.start_action(ACTIONS[self.path])
  if self.path=="/profile-api/actions/complete":
   return self.send(200,b'<!doctype html><html lang="vi"><meta charset="utf-8"><title>Nam Viet</title><body data-nv-action-return="1">Da tro ve ung dung.</body></html>',"text/html; charset=utf-8")
  if self.path!="/profile-api/avatar":return self.err(404,"Not found")
  p=ROOT/(key+".png")
  if not p.is_file():return self.err(404,"No avatar")
  self.send(200,p.read_bytes(),"image/png")
 def do_POST(self):
  key=self.identity()
  if not key:return self.err(401,"Unauthenticated")
  if self.path!="/profile-api/avatar":return self.err(404,"Not found")
  if self.headers.get("Origin")!=ORIGIN:return self.err(403,"Origin denied")
  try:n=int(self.headers.get("Content-Length","0"))
  except ValueError:return self.err(400,"Invalid length")
  if n<1 or n>LIMIT:return self.err(413,"Ảnh tối đa 2 MB")
  if self.headers.get("Content-Type") not in ("image/png","image/jpeg","image/webp"):return self.err(415,"Chỉ hỗ trợ PNG, JPEG, WebP")
  try:
   with Image.open(io.BytesIO(self.rfile.read(n)),formats=["PNG","JPEG","WEBP"]) as im:
    if im.width*im.height>12000000:raise ValueError()
    im.load(); out=ImageOps.fit(ImageOps.exif_transpose(im).convert("RGB"),(256,256),Image.Resampling.LANCZOS)
    with tempfile.NamedTemporaryFile(dir=ROOT,delete=False) as f:
     tmp=f.name
     try:out.save(f,format="PNG");f.flush();os.fsync(f.fileno())
     except Exception:os.unlink(tmp);raise
    os.replace(tmp,ROOT/(key+".png"))
  except Exception:return self.err(400,"Ảnh không hợp lệ")
  self.send(200,b'{"ok":true}')
if __name__=="__main__":ThreadingHTTPServer(("127.0.0.1",18192),Handler).serve_forever()
