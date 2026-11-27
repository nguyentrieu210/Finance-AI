import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

test('E05 evidence details use the authenticated generated API contract', async () => {
  const viewer = await readFile(new URL('../src/features/workflow/EvidenceViewer.tsx', import.meta.url), 'utf8');
  const checks = await readFile(new URL('../src/features/workflow/Checks.tsx', import.meta.url), 'utf8');
  for (const required of ["operations['getEvidence']", "'/v1/evidence/'", "cache: 'no-store'", "controller.abort()", "evidence.id !== id"]) {
    assert.ok(viewer.includes(required));
  }
  assert.ok(checks.includes('row.evidence_ids.map'));
  assert.ok(checks.includes('<EvidenceViewer'));
});
