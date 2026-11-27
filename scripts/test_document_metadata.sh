#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Reuse only the disposable CI database created by test_prototype_foundation.sh.
cid="$1"
psql_ci() { docker exec -i "$cid" psql -X -qAt -U finance_migrator -d finance_prototype -v ON_ERROR_STOP=1 "$@"; }
psql_ci -f - < scripts/document_metadata_assert.sql
# Three 25 MiB reservations, then two concurrent requests for the last 25 MiB.
sql="SELECT finance.prototype_reserve_upload('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a',gen_random_uuid(),'00000000-0000-0000-0000-000000009010','00000000-0000-0000-0000-000000009011','00000000-0000-0000-0000-0000000003e9','synthetic.pdf',26214400,decode(repeat('aa',32),'hex'),'application/pdf');"
for _ in 1 2 3; do psql_ci -c "$sql" >/dev/null; done
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
(psql_ci -c "$sql" >"$tmp/a" 2>"$tmp/a.err") & a=$!
(psql_ci -c "$sql" >"$tmp/b" 2>"$tmp/b.err") & b=$!
ra=0; rb=0
wait "$a" || ra=$?
wait "$b" || rb=$?
if [[ "$ra" == 0 && "$rb" != 0 ]]; then grep -q 'quota exceeded' "$tmp/b.err"
elif [[ "$rb" == 0 && "$ra" != 0 ]]; then grep -q 'quota exceeded' "$tmp/a.err"
else echo "quota race unexpected exits: $ra / $rb" >&2; exit 1; fi
count="$(psql_ci -c "SELECT count(*) FROM finance.upload_intent WHERE dossier_id='00000000-0000-0000-0000-000000009010' AND state='PENDING_UPLOAD';")"
[[ "$count" == 4 ]]
echo 'PASS: persisted upload metadata, generation fencing, TTL, quota release, concurrent quota reservation and denied runtime access'
