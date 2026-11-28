import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

test('E15 checklist uses the generated contract and current dossier detail', async () => {
  const panel = await readFile(new URL('../src/features/workflow/Checklist.tsx', import.meta.url), 'utf8');
  const detail = await readFile(new URL('../src/features/workflow/Drafts.tsx', import.meta.url), 'utf8');
  assert.ok(panel.includes("operations['getChecklist']"));
  assert.ok(panel.includes("'/checklist?'"));
  assert.ok(panel.includes("cache: 'no-store'"));
  assert.ok(panel.includes('controller.signal.aborted'));
  assert.ok(panel.includes('không kết luận hồ sơ đã đầy đủ'));
  for (const state of ['PRESENT', 'MISSING', 'INVALID', 'UNKNOWN', 'NOT_APPLICABLE']) {
    assert.ok(panel.includes(state + ':'));
  }
  assert.ok(detail.includes("detail.id+':checklist:'+detail.version"));
});
