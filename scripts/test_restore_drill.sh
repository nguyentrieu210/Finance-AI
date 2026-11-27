#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

image='pgvector/pgvector:0.8.0-pg17@sha256:40b404964359299eefdd5f8518facf1886c562848cf4de13b6eaf91cb70c2b87'
migrator="${FINANCE_MIGRATOR_BIN:-target/debug/finance-migrate}"
test -x "$migrator"
work="$(mktemp -d)"
password="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
cid="$(docker run -d --rm --cpus 1 --memory 768m --shm-size 128m --tmpfs /var/lib/postgresql/data:rw,size=512m -p 127.0.0.1::5432 -e POSTGRES_USER=finance_migrator -e POSTGRES_DB=finance_restore_source -e POSTGRES_PASSWORD="$password" "$image")"
trap 'rm -rf "$work"; docker stop --time 5 "$cid" >/dev/null 2>&1 || true' EXIT

for attempt in $(seq 1 60); do
  if docker exec "$cid" pg_isready -h 127.0.0.1 -U finance_migrator -d finance_restore_source >/dev/null 2>&1; then break; fi
  sleep 1
done
docker exec "$cid" pg_isready -h 127.0.0.1 -U finance_migrator -d finance_restore_source
port="$(docker port "$cid" 5432/tcp)"
[[ "$port" =~ ^127\.0\.0\.1:[0-9]+$ ]]
source_url="postgresql://finance_migrator:$password@$port/finance_restore_source?sslmode=disable"
restore_url="postgresql://finance_migrator:$password@$port/finance_restore?sslmode=disable"

FINANCE_MIGRATION_DATABASE_URL="$source_url" "$migrator" --apply
docker exec -i "$cid" psql -X -U finance_migrator -d finance_restore_source -v ON_ERROR_STOP=1 -f - < scripts/prototype_fixture.sql

# Add one chained synthetic audit event so the drill proves more than empty-schema restore.
docker exec -i "$cid" psql -X -U finance_migrator -d finance_restore_source -v ON_ERROR_STOP=1 <<'SQL'
BEGIN;
SELECT set_config('finance.tenant_id','00000000-0000-0000-0000-000000000001',true);
SELECT set_config('finance.company_id','00000000-0000-0000-0000-00000000000a',true);
SELECT set_config('finance.actor_id','00000000-0000-0000-0000-0000000003e8',true);
INSERT INTO finance.audit_event(
 tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action
) VALUES(
 '00000000-0000-0000-0000-000000000001',
 '00000000-0000-0000-0000-00000000000a',
 '00000000-0000-0000-0000-00000000f003',
 '00000000-0000-0000-0000-0000000003e8',
 '00000000-0000-0000-0000-00000000f001',
 '00000000-0000-0000-0000-00000000f002',
 '00000000-0000-0000-0000-000000000401',
 'AUDIT_READ'
);
COMMIT;
SQL

docker exec "$cid" pg_dump -U finance_migrator -d finance_restore_source -Fc --no-owner > "$work/finance.dump"
test -s "$work/finance.dump"
test "$(stat -c %s "$work/finance.dump")" -lt 67108864

docker exec "$cid" createdb -U finance_migrator finance_restore
docker exec -i "$cid" pg_restore -U finance_migrator -d finance_restore --no-owner --exit-on-error < "$work/finance.dump"

# A restored SQLx ledger must be accepted without replaying already-applied migrations.
FINANCE_MIGRATION_DATABASE_URL="$restore_url" "$migrator" --apply

docker exec -i "$cid" psql -X -U finance_migrator -d finance_restore -v ON_ERROR_STOP=1 <<'SQL'
DO $$
DECLARE head_seq bigint; max_seq bigint;
BEGIN
 IF (SELECT count(*) FROM public._sqlx_migrations WHERE success)<>41 THEN
  RAISE EXCEPTION 'restored migration ledger mismatch';
 END IF;
 IF (SELECT count(*) FROM pg_tables WHERE schemaname='finance')<>73 THEN
  RAISE EXCEPTION 'restored table inventory mismatch';
 END IF;
 IF NOT EXISTS(
  SELECT 1 FROM pg_extension e JOIN pg_namespace n ON n.oid=e.extnamespace
  WHERE e.extname='vector' AND e.extversion='0.8.0' AND n.nspname='finance'
 ) THEN RAISE EXCEPTION 'restored vector extension mismatch'; END IF;
 IF (SELECT count(*) FROM finance.tenant)<>2
 OR (SELECT count(*) FROM finance.company)<>3
 OR NOT EXISTS(SELECT 1 FROM finance.policy_version WHERE id='00000000-0000-0000-0000-000000001600' AND state='PUBLISHED')
 THEN RAISE EXCEPTION 'restored fixture mismatch'; END IF;
 IF NOT EXISTS(
  SELECT 1 FROM finance.audit_event
  WHERE command_id='00000000-0000-0000-0000-00000000f001'
   AND digest IS NOT NULL AND previous_digest IS NOT NULL
 ) THEN RAISE EXCEPTION 'restored audit event mismatch'; END IF;
 SELECT sequence INTO head_seq FROM finance.audit_chain_head
  WHERE tenant_id='00000000-0000-0000-0000-000000000001';
 SELECT max(sequence) INTO max_seq FROM finance.audit_event
  WHERE tenant_id='00000000-0000-0000-0000-000000000001';
 IF head_seq IS DISTINCT FROM max_seq THEN
  RAISE EXCEPTION 'restored audit chain head mismatch';
 END IF;
END $$;
SQL

unset source_url restore_url password
echo 'PASS: real pg_dump/pg_restore synthetic drill preserves migration ledger, schema, fixture and audit chain'
