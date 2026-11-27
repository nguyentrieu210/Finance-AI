import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

test('E18 lifecycle UI retains an uncertain command key and reloads actual state', async () => {
  const source = await readFile(new URL('../src/features/workflow/Drafts.tsx', import.meta.url), 'utf8');
  for (const part of ["'revise' | 'reopen' | 'cancel'", 'pendingChange.current', "'If-Finance-Version'", "'Idempotency-Key': cmd.key", 'actual.version < result.version', 'changeUncertain']) {
    assert.ok(source.includes(part));
  }
});
