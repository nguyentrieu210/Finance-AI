import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

test('warning acknowledgment retains FAIL and a stable retry key', async () => {
  const source = await readFile(new URL('../src/features/workflow/Checks.tsx', import.meta.url), 'utf8');
  for (const part of ["'/acknowledge'", "'Idempotency-Key': cmd.key", "'If-Finance-Version'", "receipt.outcome !== 'FAIL'", "setUncertain(true)"]) {
    assert.ok(source.includes(part));
  }
});
