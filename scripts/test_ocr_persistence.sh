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
if ! FINANCE_MIGRATION_DATABASE_URL="$url" target/debug/finance-migrate --apply; then
  # Disposable synthetic DB only; migration diagnostics contain no real records.
  docker logs "$cid" 2>&1 | grep -E 'ERROR:|HINT:|DETAIL:|LINE [0-9]+:' | tail -20 >&2 || true
  exit 1
fi
docker exec -i "$cid" psql -X -U finance_migrator -d finance_ocr -v ON_ERROR_STOP=1 -f - < scripts/prototype_fixture.sql
pw="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
printf "CREATE ROLE finance_ocr_test_worker LOGIN NOINHERIT NOSUPERUSER NOCREATEROLE NOCREATEDB NOBYPASSRLS PASSWORD '%s';\nGRANT finance_job_worker TO finance_ocr_test_worker;\nINSERT INTO finance.worker_scope(login_name,tenant_id,company_id) VALUES ('finance_ocr_test_worker','00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a');\n" "$pw" | docker exec -i "$cid" psql -X -U finance_migrator -d finance_ocr -v ON_ERROR_STOP=1 >/dev/null
worker="postgresql://finance_ocr_test_worker:$pw@$port/finance_ocr?sslmode=disable"
unset pw
trained="$(find /usr/share/tesseract-ocr -name vie.traineddata -type f -print -quit)"
test -f "$trained"
FINANCE_OCR_DB_URL="$url" FINANCE_OCR_WORKER_URL="$worker" FINANCE_OCR_TRAINEDDATA="$trained" cargo test --locked -p finance-extraction --test real_persist -- --ignored --exact real_candidate_persistence
docker exec -i "$cid" psql -X -U finance_migrator -d finance_ocr -v ON_ERROR_STOP=1 -f - < scripts/field_runtime_fixture.sql
runtime_pw="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
identity_pw="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
printf "CREATE ROLE finance_ocr_runtime_login LOGIN NOINHERIT NOSUPERUSER NOCREATEROLE NOCREATEDB NOBYPASSRLS PASSWORD '%s';\nGRANT finance_runtime TO finance_ocr_runtime_login;\nCREATE ROLE finance_ocr_identity_login LOGIN NOINHERIT NOSUPERUSER NOCREATEROLE NOCREATEDB NOBYPASSRLS PASSWORD '%s';\nGRANT finance_identity_lookup TO finance_ocr_identity_login;\n" "$runtime_pw" "$identity_pw" | docker exec -i "$cid" psql -X -U finance_migrator -d finance_ocr -v ON_ERROR_STOP=1 >/dev/null
runtime_url="postgresql://finance_ocr_runtime_login:$runtime_pw@$port/finance_ocr?sslmode=disable"
identity_url="postgresql://finance_ocr_identity_login:$identity_pw@$port/finance_ocr?sslmode=disable"
unset runtime_pw identity_pw
docker exec -i "$cid" psql -X -U finance_migrator -d finance_ocr -v ON_ERROR_STOP=1 -f - <<'SQL'
BEGIN;
SET ROLE finance_runtime;
SELECT set_config('finance.tenant_id','00000000-0000-0000-0000-000000000001',true);
SELECT set_config('finance.company_id','00000000-0000-0000-0000-00000000000a',true);
SELECT set_config('finance.actor_id','00000000-0000-0000-0000-0000000003e9',true);
DO $
DECLARE n bigint;
BEGIN
 SELECT count(*) INTO n FROM finance.runtime_fields(
  '00000000-0000-0000-0000-000000000001',
  '00000000-0000-0000-0000-00000000000a',
  '00000000-0000-0000-0000-000000001f41',
  '00000000-0000-0000-0000-0000000003e9',
  '00000000-0000-0000-0000-000000000000',
  NULL,100
 );
 IF n<>6 THEN RAISE EXCEPTION 'runtime field projection expected 6 rows, got %',n; END IF;
END $;
ROLLBACK;
SQL
FINANCE_OCR_RUNTIME_URL="$runtime_url" FINANCE_OCR_IDENTITY_URL="$identity_url" cargo test --locked -p finance-api --test fields_db -- --ignored --exact actual_ocr_fields_http_review
docker exec -i "$cid" psql -X -U finance_migrator -d finance_ocr -v ON_ERROR_STOP=1 -f - < scripts/ocr_review_preflight_assert.sql
docker exec -i "$cid" psql -X -U finance_migrator -d finance_ocr -v ON_ERROR_STOP=1 -f - < scripts/check_read_assert.sql
unset url worker runtime_url identity_url
echo 'PASS: actual OCR row candidates, immutable evidence, source digest and fenced job transaction in PostgreSQL'
