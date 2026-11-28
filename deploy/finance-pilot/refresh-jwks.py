#!/usr/bin/python3
import json, os, tempfile, subprocess
from pathlib import Path
url = "https://namviet.gpcds.site/realms/namviet/protocol/openid-connect/certs"
target = Path("/var/lib/namviet-finance-jwks/jwks.json")
raw = subprocess.check_output(["/usr/bin/curl","--fail","--silent","--show-error","--max-time","8","--max-filesize","131072","--resolve","namviet.gpcds.site:443:127.0.0.1",url])
if len(raw) > 131072:
    raise ValueError("oversize JWKS")
data = json.loads(raw)
keys = [k for k in data["keys"] if k.get("kty") == "RSA" and k.get("alg") == "RS256" and k.get("use") == "sig" and k.get("n") and k.get("e")]
if not keys:
    raise ValueError("no acceptable signing key")
# The issuer also publishes encryption keys; never offer these to the access-token verifier.
raw = json.dumps({"keys":keys}, separators=(",", ":")).encode()
with tempfile.NamedTemporaryFile(dir=target.parent, delete=False) as f:
    name = f.name
    f.write(raw)
    f.flush()
    os.fsync(f.fileno())
os.chmod(name, 0o644)
os.replace(name, target)
