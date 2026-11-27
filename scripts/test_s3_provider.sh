#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Disposable versioned object store, no host volume, no real credentials or documents.
image='docker.io/minio/minio:RELEASE.2025-04-22T22-12-26Z'
export MINIO_ROOT_USER="finance-probe-$(python3 -c 'import secrets; print(secrets.token_hex(6))')"
export MINIO_ROOT_PASSWORD="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
# Pass secrets by inherited environment, never as command arguments or logs.
cid="$(docker run --rm -d -e MINIO_ROOT_USER -e MINIO_ROOT_PASSWORD -p 127.0.0.1::9000 --tmpfs /data:rw,size=256m "$image" server /data)"
trap 'docker stop --time 5 "$cid" >/dev/null 2>&1 || true' EXIT
address="$(docker port "$cid" 9000/tcp)"
[[ "$address" =~ ^127\.0\.0\.1:[0-9]+$ ]]
export FINANCE_S3_PROBE_ENDPOINT="http://$address"
for attempt in $(seq 1 60); do
  if curl --silent --fail --max-time 2 "$FINANCE_S3_PROBE_ENDPOINT/minio/health/ready" >/dev/null; then break; fi
  sleep 1
done
curl --silent --fail --max-time 5 "$FINANCE_S3_PROBE_ENDPOINT/minio/health/ready" >/dev/null
docker image inspect "$image" --format 'S3 probe image ID: {{.Id}}'
"${FINANCE_PROBE_PYTHON:-.venv/bin/python}" scripts/s3_provider_probe.py
unset MINIO_ROOT_USER MINIO_ROOT_PASSWORD FINANCE_S3_PROBE_ENDPOINT
