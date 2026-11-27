#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Clamd is installed by GitHub Actions, never on VPS. Synthetic local signature only.
work="$(mktemp -d)"
pid=""
cleanup() {
  if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
  rm -rf "$work"
}
trap cleanup EXIT
mkdir "$work/db"
python3 - "$work/db/synthetic.ndb" <<'PY'
from pathlib import Path
import sys
Path(sys.argv[1]).write_text("Finance.SyntheticProbe:0:*:" + b"FINANCE_SYNTHETIC_SCAN_PROBE_2026".hex() + "\n")
PY
port="$(python3 - <<'PY'
import socket
with socket.socket() as s:
    s.bind(("127.0.0.1", 0))
    print(s.getsockname()[1])
PY
)"
cat > "$work/clamd.conf" <<EOF
Foreground yes
DatabaseDirectory $work/db
TCPSocket $port
TCPAddr 127.0.0.1
StreamMaxLength 25M
MaxFileSize 25M
MaxScanSize 25M
MaxThreads 2
ReadTimeout 15
CommandReadTimeout 15
SelfCheck 0
EOF
clamd --version
clamd --config-file="$work/clamd.conf" >"$work/clamd.log" 2>&1 &
pid=$!
python3 - "$port" <<'PY'
import socket, sys, time
port = int(sys.argv[1])
for _ in range(60):
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=1) as s:
            s.sendall(b"zPING\0")
            if s.recv(32) == b"PONG\0":
                break
    except OSError:
        pass
    time.sleep(1)
else:
    raise SystemExit("isolated clamd failed readiness")
PY
FINANCE_CLAMD_PROBE_ADDRESS="127.0.0.1:$port" cargo test --locked -p finance-documents --test scan_db -- --ignored --exact real_clamd_synthetic_signatures
echo 'PASS: Rust INSTREAM adapter with actual ClamAV engine and local synthetic signature; not a current malware database'
