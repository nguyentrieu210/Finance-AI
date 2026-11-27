from pathlib import Path
import json,re
root=Path(__file__).resolve().parents[1]
r=json.loads((root/'migrations/registry/table-ownership.json').read_text())
assert r['status']=='TECHNICAL_DRAFT'
assert len(r['groups'])==24
assert {g['owner'] for g in r['groups']}=={f'A{i:02d}' for i in range(1,25)}
tables=[t for g in r['groups'] for t in g['tables']]
assert len(tables)==len(set(tables))
assert all(re.fullmatch(r'[a-z][a-z0-9_]*',t) for t in tables)
spec=(root/'specs/001-financial-control/data-model.md').read_text()
expected=set()
for line in spec.splitlines():
    if line.startswith('| ') and not line.startswith('| Entity') and not line.startswith('|---'):
        for t in line.split('|')[1].strip().split(' / '):
            assert re.fullmatch(r'[a-z][a-z0-9_]*',t),t
            expected.add(t)
assert expected<=set(tables),sorted(expected-set(tables))
assert {'upload_intent','upload_grant'}<=set(tables)
assert all(g['invariants'] for g in r['groups'])
assert (root/'migrations'/f"{r['migration_baseline']}_foundation.sql").exists()
print(f'PASS: {len(tables)} planned tables across 24 owners, all {len(expected)} logical spec entities covered; no DDL/signoff implied.')
