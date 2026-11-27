#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
image='pgvector/pgvector:0.8.0-pg17@sha256:40b404964359299eefdd5f8518facf1886c562848cf4de13b6eaf91cb70c2b87'
# Disposable isolated database. No host port, host volume, or existing DB connection.
password="$(python3 -c 'import secrets; print(secrets.token_hex(24))')"
cid="$(docker run -d --rm --network none --cpus 1 --memory 512m --shm-size 128m --tmpfs /var/lib/postgresql/data:rw,size=384m -e POSTGRES_USER=finance_migrator -e POSTGRES_DB=finance_baseline -e POSTGRES_PASSWORD="$password" "$image")"
unset password
trap 'docker stop --time 5 "$cid" >/dev/null 2>&1 || true' EXIT
for attempt in $(seq 1 60); do
  if docker exec "$cid" pg_isready -h 127.0.0.1 -U finance_migrator -d finance_baseline >/dev/null 2>&1; then break; fi
  sleep 1
done
docker exec "$cid" pg_isready -h 127.0.0.1 -U finance_migrator -d finance_baseline
docker exec -i "$cid" psql -X -U finance_migrator -d finance_baseline -v ON_ERROR_STOP=1 --single-transaction -f - < migrations/20261127000100_foundation.sql
docker exec -i "$cid" psql -X -U finance_migrator -d finance_baseline -v ON_ERROR_STOP=1 <<'SQL'
CREATE ROLE finance_probe NOLOGIN;
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace WHERE e.extname = 'vector' AND n.nspname = 'finance' AND e.extversion = '0.8.0') THEN
    RAISE EXCEPTION 'vector extension missing/wrong schema/version';
  END IF;
  IF has_schema_privilege('finance_probe', 'finance', 'USAGE') OR has_schema_privilege('finance_probe', 'finance', 'CREATE') THEN
    RAISE EXCEPTION 'unprivileged role has finance schema access';
  END IF;
  IF EXISTS (SELECT 1 FROM pg_tables WHERE schemaname = 'finance') THEN
    RAISE EXCEPTION 'unexpected business table at G01 baseline';
  END IF;
END $$;
SELECT current_setting('server_version') AS postgres_version;
SQL
echo 'PASS: baseline migration and private schema on real isolated PostgreSQL/pgvector'
