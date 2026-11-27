#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Disposable synthetic DB, no existing volume or user credentials.
image='pgvector/pgvector:0.8.0-pg17@sha256:40b404964359299eefdd5f8518facf1886c562848cf4de13b6eaf91cb70c2b87'
password="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
cid="$(docker run -d --rm --cpus 1 --memory 512m --shm-size 128m --tmpfs /var/lib/postgresql/data:rw,size=384m -p 127.0.0.1::5432 -e POSTGRES_USER=finance_migrator -e POSTGRES_DB=finance_ocr -e POSTGRES_PASSWORD="$password" "$image")"
trap 'docker stop --time 5 "$cid" >/dev/null 2>&1 || true' EXIT
for attempt in $(seq 1 60); do
  if docker exec "$cid" pg_isready -h 127.0.0.1 -U finance_migrator -d finance_ocr >/dev/null 2>&1; then break; fi
  sleep 1
done
docker exec "$cid" pg_isready -h 127.0.0.1 -U finance_migrator -d finance_ocr
port="$(docker port "$cid" 5432/tcp)"
[[ "$port" =~ ^127\.0\.0\.1:[0-9]+$ ]]
url="postgresql://finance_migrator:$password@$port/finance_ocr?sslmode=disable"
unset password
cargo build --locked -p finance-migrate
FINANCE_MIGRATION_DATABASE_URL="$url" target/debug/finance-migrate --apply
docker exec -i "$cid" psql -X -U finance_migrator -d finance_ocr -v ON_ERROR_STOP=1 -f - < scripts/prototype_fixture.sql
pw="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
printf "CREATE ROLE finance_ocr_test_worker LOGIN NOINHERIT NOSUPERUSER NOCREATEROLE NOCREATEDB NOBYPASSRLS PASSWORD '%s';\nGRANT finance_job_worker TO finance_ocr_test_worker;\nINSERT INTO finance.worker_scope(login_name,tenant_id,company_id) VALUES ('finance_ocr_test_worker','00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a');\n" "$pw" | docker exec -i "$cid" psql -X -U finance_migrator -d finance_ocr -v ON_ERROR_STOP=1 >/dev/null
worker="postgresql://finance_ocr_test_worker:$pw@$port/finance_ocr?sslmode=disable"
unset pw
trained="$(find /usr/share/tesseract-ocr -name vie.traineddata -type f -print -quit)"
test -f "$trained"
FINANCE_OCR_DB_URL="$url" FINANCE_OCR_WORKER_URL="$worker" FINANCE_OCR_TRAINEDDATA="$trained" cargo test --locked -p finance-extraction --test real_persist -- --ignored --exact real_candidate_persistence
unset url worker
echo 'PASS: actual OCR row candidates, immutable evidence, source digest and fenced job transaction in PostgreSQL'
