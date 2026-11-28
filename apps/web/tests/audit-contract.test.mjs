import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

test('audit list labels its page-local row position without implying a chain sequence', async () => {
  const view = await readFile(new URL('../src/features/audit/AuditLog.tsx', import.meta.url), 'utf8');
  const contract = await readFile(new URL('../../../contracts/openapi.yaml', import.meta.url), 'utf8');
  const auditSchema = contract.match(/^    Audit:\n([\s\S]*?)(?=^    [A-Z][A-Za-z0-9]*:\n)/m)?.[1];

  assert.ok(auditSchema, 'the OpenAPI Audit schema exists');
  assert.doesNotMatch(auditSchema, /^\s{8}sequence:/im, 'the API does not expose a per-tenant chain sequence');
  assert.match(view, /<th>Vị trí trong trang<\/th>/);
  assert.match(view, /<td>\{index\+1\}<\/td>/);
  assert.doesNotMatch(view, /<th>Sequence<\/th>/i);
});
