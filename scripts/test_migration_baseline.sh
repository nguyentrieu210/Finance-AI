#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
image='pgvector/pgvector:0.8.0-pg17@sha256:40b404964359299eefdd5f8518facf1886c562848cf4de13b6eaf91cb70c2b87'
migrator="${FINANCE_MIGRATOR_BIN:-target/debug/finance-migrate}"
test -x "$migrator"
# Disposable local-only database, randomly assigned loopback port and password.
# No host data volume, existing database, or production credential.
password="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
cid="$(docker run -d --rm --cpus 1 --memory 512m --shm-size 128m --tmpfs /var/lib/postgresql/data:rw,size=384m -p 127.0.0.1::5432 -e POSTGRES_USER=finance_migrator -e POSTGRES_DB=finance_baseline -e POSTGRES_PASSWORD="$password" "$image")"
trap 'docker stop --time 5 "$cid" >/dev/null 2>&1 || true' EXIT
for attempt in $(seq 1 60); do
  if docker exec "$cid" pg_isready -h 127.0.0.1 -U finance_migrator -d finance_baseline >/dev/null 2>&1; then break; fi
  sleep 1
done
docker exec "$cid" pg_isready -h 127.0.0.1 -U finance_migrator -d finance_baseline
port="$(docker port "$cid" 5432/tcp)"
[[ "$port" =~ ^127\.0\.0\.1:[0-9]+$ ]]
url="postgresql://finance_migrator:$password@$port/finance_baseline?sslmode=disable"
unset password
# Require explicit command and credentials, then run twice to prove SQLx ledger replay.
if "$migrator"; then echo 'migrator accepted missing --apply' >&2; exit 1; fi
if env -u FINANCE_MIGRATION_DATABASE_URL "$migrator" --apply; then echo 'migrator accepted missing URL' >&2; exit 1; fi
if ! FINANCE_MIGRATION_DATABASE_URL="$url" "$migrator" --apply; then
  # Disposable synthetic DB only; migration diagnostics contain no real records.
  docker logs "$cid" 2>&1 | grep -E 'ERROR:|HINT:|DETAIL:|LINE [0-9]+:' | tail -20 >&2 || true
  exit 1
fi
FINANCE_MIGRATION_DATABASE_URL="$url" "$migrator" --apply
docker exec -i "$cid" psql -X -U finance_migrator -d finance_baseline -v ON_ERROR_STOP=1 <<'SQL'
CREATE ROLE finance_probe NOLOGIN;
DO $$
BEGIN
  IF (SELECT count(*) FROM public._sqlx_migrations) <> 29 OR NOT EXISTS (
    SELECT 1 FROM public._sqlx_migrations WHERE version = 20261127000100 AND success AND octet_length(checksum) = 48
  ) THEN RAISE EXCEPTION 'migration ledger mismatch'; END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace WHERE e.extname = 'vector' AND n.nspname = 'finance' AND e.extversion = '0.8.0') THEN
    RAISE EXCEPTION 'vector extension missing/wrong schema/version';
  END IF;
  IF has_schema_privilege('finance_probe', 'finance', 'USAGE') OR has_schema_privilege('finance_probe', 'finance', 'CREATE') OR has_table_privilege('finance_probe','public._sqlx_migrations','UPDATE') THEN
    RAISE EXCEPTION 'unprivileged role has schema/ledger write access';
  END IF;
  IF (SELECT count(*) FROM pg_tables WHERE schemaname = 'finance') <> 70 THEN
    RAISE EXCEPTION 'unexpected prototype table inventory';
  END IF;
END $$;
SELECT current_setting('server_version') AS postgres_version;
-- Simulate tampering in this disposable database only; the migrator must reject drift.
UPDATE public._sqlx_migrations SET checksum = decode(repeat('00',48),'hex') WHERE version=20261127000100;
SQL
if FINANCE_MIGRATION_DATABASE_URL="$url" "$migrator" --apply; then echo 'migrator accepted checksum drift' >&2; exit 1; fi
unset url
echo 'PASS: SQLx migration, replay, checksum drift rejection, private schema on real isolated PostgreSQL/pgvector'
