#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
image="finance-paddle-probe:${GITHUB_RUN_ID:-local}"
work="$(mktemp -d)"
trap 'rm -rf "$work"; docker image rm "$image" >/dev/null 2>&1 || true' EXIT
docker build --tag "$image" tests/ocr/paddle
# Only model assets are downloaded during the image build. The inference
# container has no network, credentials, host mounts, or writable root.
(
  ulimit -f 4096
  timeout --kill-after=5s 180s docker run --rm --network none --read-only \
    --cap-drop ALL --security-opt no-new-privileges --user 65534:65534 \
    --cpus 2 --memory 3g --pids-limit 128 --tmpfs /tmp:rw,nosuid,nodev,size=256m,mode=1777 \
    "$image" > "$work/result.json"
)
FINANCE_PADDLE_JSON="$work/result.json" cargo test --locked -p finance-ocr --test real_paddle -- --ignored --exact real_paddle_vietnamese
echo 'PASS: real isolated PaddleOCR PP-OCRv5 Mobile CPU synthetic inference -> Rust candidate geometry'
