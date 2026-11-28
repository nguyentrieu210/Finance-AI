#!/usr/bin/env bash
set -euo pipefail
cid="$1"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
psql_cmd=(docker exec -i "$cid" psql -X -U finance_migrator -d finance_prototype -v ON_ERROR_STOP=1)
"${psql_cmd[@]}" <<'SQL'
INSERT INTO finance.financial_resource VALUES
 ('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000001101','VND',100,0,true,clock_timestamp()+interval '1 hour'),
 ('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000001102','VND',100,0,true,clock_timestamp()+interval '1 hour'),
 ('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-000000001103','VND',10,0,true,clock_timestamp()+interval '1 hour');
SQL
reserve() {
 local id="$1" amount="$2" resources="$3"
 "${psql_cmd[@]}" -c "SELECT finance.prototype_reserve_finance('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-00000000$id','00000000-0000-0000-0000-000000001200','VND',$amount,ARRAY[$resources]::uuid[]);"
}
a="'00000000-0000-0000-0000-000000001101'"
b="'00000000-0000-0000-0000-000000001102'"
c="'00000000-0000-0000-0000-000000001103'"
# Independent connections, opposite resource order; only one 60/100 hold wins.
reserve 1301 60 "$a,$b" >"$work/one" 2>&1 & p1=$!
reserve 1302 60 "$b,$a" >"$work/two" 2>&1 & p2=$!
s1=0; wait "$p1" || s1=$?
s2=0; wait "$p2" || s2=$?
if [[ "$s1" == 0 && "$s2" != 0 ]]; then winner=1301; grep -q 'cap exceeded' "$work/two"
elif [[ "$s2" == 0 && "$s1" != 0 ]]; then winner=1302; grep -q 'cap exceeded' "$work/one"
else echo 'expected exactly one concurrent reservation success' >&2; exit 1; fi
reserve "$winner" 60 "$b,$a" >/dev/null
if reserve "$winner" 61 "$a,$b" >"$work/mismatch" 2>&1; then exit 1; fi
grep -q 'reservation replay mismatch' "$work/mismatch"
if reserve 1303 20 "$a,$c" >"$work/rollback" 2>&1; then exit 1; fi
grep -q 'cap exceeded' "$work/rollback"
"${psql_cmd[@]}" <<'SQL'
DO $$
BEGIN
 IF (SELECT count(*) FROM finance.reservation)<>1 OR (SELECT count(*) FROM finance.reservation_line)<>2
 OR (SELECT sum(amount) FROM finance.reservation_line)<>120 THEN
  RAISE EXCEPTION 'reservation atomicity failed';
 END IF;
 IF has_function_privilege('finance_runtime','finance.prototype_reserve_finance(uuid,uuid,uuid,uuid,text,numeric,uuid[])','EXECUTE')
 OR has_table_privilege('finance_runtime','finance.reservation','INSERT') THEN
  RAISE EXCEPTION 'synthetic reservation exposed to runtime';
 END IF;
END $$;
SQL
echo 'PASS: real concurrent multi-resource reservation, replay, atomic rollback and default-deny'
