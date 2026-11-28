#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# GitHub-hosted synthetic-only renderer probe. No bind mounts, Docker socket,
# secrets or external network during document processing.
image="finance-pdf-render-probe:${GITHUB_RUN_ID:-local}"
docker build --tag "$image" tests/ocr/render
trap 'docker image rm "$image" >/dev/null 2>&1 || true' EXIT
timeout --kill-after=5s 150s docker run --rm --network none --read-only \
  --cap-drop ALL --security-opt no-new-privileges --user 65534:65534 \
  --cpus 1 --memory 512m --pids-limit 32 --tmpfs /tmp:rw,noexec,nosuid,nodev,size=96m,mode=1777 \
  "$image"
echo 'PASS: actual bounded Poppler PDF renderer, text layer, rotation, malformed and page limit probes'
