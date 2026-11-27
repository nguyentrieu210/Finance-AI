#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
image="finance-office-export:${GITHUB_RUN_ID:-local}"
work="$(mktemp -d)"
trap 'rm -rf "$work"; docker image rm "$image" >/dev/null 2>&1 || true' EXIT
docker build --tag "$image" tests/export
(
 ulimit -f 8192
 timeout --kill-after=5s 120s docker run --rm --network none --read-only \
 --cap-drop ALL --security-opt no-new-privileges --user 65534:65534 \
 --cpus 2 --memory 1024m --pids-limit 64 --tmpfs /tmp:rw,nosuid,nodev,size=128m,mode=1777 \
 "$image" > "$work/result.json"
)
FINANCE_EXPORT_JSON="$work/result.json" cargo test --locked -p finance-templates --test real_export -- --ignored --exact real_office_export
echo 'PASS: actual DOCX, XLSX, LibreOffice PDF and Rust manifest readback'
