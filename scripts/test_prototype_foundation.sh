#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
image='pgvector/pgvector:0.8.0-pg17@sha256:40b404964359299eefdd5f8518facf1886c562848cf4de13b6eaf91cb70c2b87'
migrator="${FINANCE_MIGRATOR_BIN:-target/debug/finance-migrate}"
test -x "$migrator"
password="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
cid="$(docker run -d --rm --cpus 1 --memory 512m --shm-size 128m --tmpfs /var/lib/postgresql/data:rw,size=384m -p 127.0.0.1::5432 -e POSTGRES_USER=finance_migrator -e POSTGRES_DB=finance_prototype -e POSTGRES_PASSWORD="$password" "$image")"
trap 'docker stop --time 5 "$cid" >/dev/null 2>&1 || true' EXIT
for attempt in $(seq 1 60); do
  if docker exec "$cid" pg_isready -h 127.0.0.1 -U finance_migrator -d finance_prototype >/dev/null 2>&1; then break; fi
  sleep 1
done
docker exec "$cid" pg_isready -h 127.0.0.1 -U finance_migrator -d finance_prototype
port="$(docker port "$cid" 5432/tcp)"
[[ "$port" =~ ^127\.0\.0\.1:[0-9]+$ ]]
url="postgresql://finance_migrator:$password@$port/finance_prototype?sslmode=disable"
unset password
FINANCE_MIGRATION_DATABASE_URL="$url" "$migrator" --apply
docker exec -i "$cid" psql -X -U finance_migrator -d finance_prototype -v ON_ERROR_STOP=1 -f - < scripts/prototype_fixture.sql
runtime_pw="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
# Ephemeral non-privileged login in this disposable DB. Never share migration credentials.
printf "CREATE ROLE finance_test_login LOGIN NOINHERIT NOSUPERUSER NOCREATEROLE NOCREATEDB NOBYPASSRLS PASSWORD '%s';\nGRANT finance_runtime TO finance_test_login;\n" "$runtime_pw" | docker exec -i "$cid" psql -X -U finance_migrator -d finance_prototype -v ON_ERROR_STOP=1 >/dev/null
runtime_url="postgresql://finance_test_login:$runtime_pw@$port/finance_prototype?sslmode=disable"
unset runtime_pw
if [[ -n "${FINANCE_PROTOTYPE_TEST_BIN:-}" ]]; then
 FINANCE_PROTOTYPE_TEST_DATABASE_URL="$runtime_url" "$FINANCE_PROTOTYPE_TEST_BIN" --ignored --exact real_database_transaction_and_scope
else
 FINANCE_PROTOTYPE_TEST_DATABASE_URL="$runtime_url" cargo test --locked -p finance-core --test prototype_db -- --ignored --exact real_database_transaction_and_scope
fi
docker exec -i "$cid" psql -X -U finance_migrator -d finance_prototype -v ON_ERROR_STOP=1 -f - < scripts/prototype_assert.sql
worker_pw="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
printf "CREATE ROLE finance_job_test_login LOGIN NOINHERIT NOSUPERUSER NOCREATEROLE NOCREATEDB NOBYPASSRLS PASSWORD '%s';\nGRANT finance_job_worker TO finance_job_test_login;\nINSERT INTO finance.worker_scope(login_name,tenant_id,company_id) VALUES ('finance_job_test_login','00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a');\n" "$worker_pw" | docker exec -i "$cid" psql -X -U finance_migrator -d finance_prototype -v ON_ERROR_STOP=1 >/dev/null
worker_url="postgresql://finance_job_test_login:$worker_pw@$port/finance_prototype?sslmode=disable"
unset worker_pw
if [[ -n "${FINANCE_JOBS_TEST_BIN:-}" ]]; then
 FINANCE_JOB_TEST_DATABASE_URL="$worker_url" "$FINANCE_JOBS_TEST_BIN" --ignored --exact durable_jobs_real_database
else
 FINANCE_JOB_TEST_DATABASE_URL="$worker_url" cargo test --locked -p finance-core --test jobs_db -- --ignored --exact durable_jobs_real_database
fi
docker exec -i "$cid" psql -X -U finance_migrator -d finance_prototype -v ON_ERROR_STOP=1 -f - < scripts/jobs_assert.sql
if [[ -n "${FINANCE_AUDIT_TEST_BIN:-}" ]]; then
 FINANCE_PROTOTYPE_TEST_DATABASE_URL="$runtime_url" FINANCE_AUDIT_TEST_ADMIN_URL="$url" "$FINANCE_AUDIT_TEST_BIN" --ignored --exact scoped_audit_real_database
else
 FINANCE_PROTOTYPE_TEST_DATABASE_URL="$runtime_url" FINANCE_AUDIT_TEST_ADMIN_URL="$url" cargo test --locked -p finance-audit --test audit_db -- --ignored --exact scoped_audit_real_database
fi
identity_pw="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
printf "CREATE ROLE finance_identity_test_login LOGIN NOINHERIT NOSUPERUSER NOCREATEROLE NOCREATEDB NOBYPASSRLS PASSWORD '%s';\nGRANT finance_identity_lookup TO finance_identity_test_login;\n" "$identity_pw" | docker exec -i "$cid" psql -X -U finance_migrator -d finance_prototype -v ON_ERROR_STOP=1 >/dev/null
identity_url="postgresql://finance_identity_test_login:$identity_pw@$port/finance_prototype?sslmode=disable"
unset identity_pw
if [[ -n "${FINANCE_IDENTITY_TEST_BIN:-}" ]]; then
 FINANCE_IDENTITY_TEST_DATABASE_URL="$identity_url" FINANCE_AUDIT_TEST_ADMIN_URL="$url" "$FINANCE_IDENTITY_TEST_BIN" --ignored --exact tests::identity_real_database
else
 FINANCE_IDENTITY_TEST_DATABASE_URL="$identity_url" FINANCE_AUDIT_TEST_ADMIN_URL="$url" cargo test --locked -p finance-identity --lib -- --ignored --exact tests::identity_real_database
fi
if [[ -n "${FINANCE_PERMISSION_TEST_BIN:-}" ]]; then
 FINANCE_PROTOTYPE_TEST_DATABASE_URL="$runtime_url" FINANCE_AUDIT_TEST_ADMIN_URL="$url" "$FINANCE_PERMISSION_TEST_BIN" --ignored --exact permission_revocation_real_database
else
 FINANCE_PROTOTYPE_TEST_DATABASE_URL="$runtime_url" FINANCE_AUDIT_TEST_ADMIN_URL="$url" cargo test --locked -p finance-core --test permission_db -- --ignored --exact permission_revocation_real_database
fi
if [[ -n "${FINANCE_ORG_TEST_BIN:-}" ]]; then
 FINANCE_PROTOTYPE_TEST_DATABASE_URL="$runtime_url" FINANCE_AUDIT_TEST_ADMIN_URL="$url" "$FINANCE_ORG_TEST_BIN" --ignored --exact organization_real_database
else
 FINANCE_PROTOTYPE_TEST_DATABASE_URL="$runtime_url" FINANCE_AUDIT_TEST_ADMIN_URL="$url" cargo test --locked -p finance-organization --test lifecycle_db -- --ignored --exact organization_real_database
fi
if [[ -n "${FINANCE_BINDING_TEST_BIN:-}" ]]; then
 FINANCE_PROTOTYPE_TEST_DATABASE_URL="$runtime_url" FINANCE_IDENTITY_TEST_DATABASE_URL="$identity_url" FINANCE_AUDIT_TEST_ADMIN_URL="$url" "$FINANCE_BINDING_TEST_BIN" --ignored --exact authenticated_commands_real_database
else
 FINANCE_PROTOTYPE_TEST_DATABASE_URL="$runtime_url" FINANCE_IDENTITY_TEST_DATABASE_URL="$identity_url" FINANCE_AUDIT_TEST_ADMIN_URL="$url" cargo test --locked -p finance-organization --test binding_db -- --ignored --exact authenticated_commands_real_database
fi
unset url runtime_url worker_url identity_url
echo 'PASS: real PostgreSQL synthetic prototype atomic command, replay, RLS and scoped FK'
