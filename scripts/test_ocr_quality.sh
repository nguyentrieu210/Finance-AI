#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
image="finance-ocr-quality:${GITHUB_RUN_ID:-local}"
docker build --tag "$image" tests/ocr/quality
trap 'docker image rm "$image" >/dev/null 2>&1 || true' EXIT
timeout --kill-after=5s 120s docker run --rm --network none --read-only \
  --cap-drop ALL --security-opt no-new-privileges --user 65534:65534 \
  --cpus 1 --memory 768m --pids-limit 32 --tmpfs /tmp:rw,noexec,nosuid,nodev,size=64m,mode=1777 \
  "$image"
echo 'PASS: synthetic grayscale quality metrics, bounded deskew, inverse affine mapping and negative probes'
