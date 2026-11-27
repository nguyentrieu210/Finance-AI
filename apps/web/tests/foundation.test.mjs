import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

test('foundation does not expose an upload or payment form', async () => {
  const src = await readFile(new URL('../src/main.tsx', import.meta.url), 'utf8');
  assert.match(src, /Chưa tiếp nhận chứng từ/);
  assert.doesNotMatch(src, /<form|type=["']file["']/);
});
