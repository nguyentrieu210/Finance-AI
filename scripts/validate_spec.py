from pathlib import Path
import yaml, json
from collections import Counter
from openapi_parameters import HTTP_METHODS, validate_path_item_parameters
R=Path(__file__).resolve().parents[1]
def load(p): return yaml.safe_load((R/p).read_text())
t=load('agents/task-graph.yaml'); q=load('tests/requirements.yaml'); a=load('tests/acceptance.yaml'); api=load('contracts/openapi.yaml')
ids={x['id'] for x in t}; assert len(ids)==len(t)
assert all(set(x['depends'])<=ids and x['id'] not in x['depends'] for x in t)
remaining={x['id']:set(x['depends']) for x in t}; waves=[]
while remaining:
    ready={k for k,v in remaining.items() if not v}; assert ready, 'cycle'
    waves.append(sorted(ready)); remaining={k:v-ready for k,v in remaining.items() if k not in ready}
req={x['id'] for x in q}; ats={x['id'] for x in a}; assert len(req)==len(q) and len(ats)==len(a)
assert all(x['test'] in ats for x in q)
assert all(set(x['requirements'])<=req for x in t)
assert req=={i for x in t for i in x['requirements']}
assert all(x['requirement'] in req|{'SYSTEM'} for x in a)
ops=[v for p in api['paths'].values() for k,v in p.items() if k in HTTP_METHODS]
assert len({x['operationId'] for x in ops})==len(ops)
def walk(v):
    if isinstance(v,dict):
        assert v.get('additionalProperties') is not True
        if '$ref' in v:
            s=v['$ref']; assert (s.startswith('#/components/schemas/') and s.split('/')[-1] in api['components']['schemas']) or (s=='./rule-dsl.schema.json' and (R/'contracts/rule-dsl.schema.json').exists()),s
        for x in v.values():walk(x)
    elif isinstance(v,list):
        for x in v:walk(x)
walk(api)
for path,path_item in api['paths'].items():
    validate_path_item_parameters(path, path_item)
try:
    from jsonschema import Draft202012Validator
    Draft202012Validator.check_schema(json.loads((R/'contracts/rule-dsl.schema.json').read_text()))
    dsl='validated schema syntax'
except ImportError: dsl='jsonschema unavailable; not validated'
print(f'PASS static checks: {len(t)} tasks, {len(waves)} topological levels, max ready set {max(map(len,waves))}, {len(q)} module requirements, {len(a)} acceptance cases, {len(ops)} API operations / {len(api["paths"])} paths; DSL {dsl}.')
print('No code, runtime tests, full OpenAPI validator or business signoff performed.')
