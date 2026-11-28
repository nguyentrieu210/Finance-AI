import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
test('product workspace does not render a generic page introduction',async()=>{
 const code=await readFile(new URL('../src/product/Workspace.tsx',import.meta.url),'utf8');
 const css=await readFile(new URL('../src/product/workspace.css',import.meta.url),'utf8');
 assert.doesNotMatch(code,/nv-page-title|KHÔNG GIAN LÀM VIỆC|Quản lý và kiểm soát hồ sơ tài chính Nam Việt\./);
 assert.doesNotMatch(css,/nv-page-title/);
});
