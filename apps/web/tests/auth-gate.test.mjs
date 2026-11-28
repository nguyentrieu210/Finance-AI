import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
test('OIDC sign-in is delegated to the same-origin gateway and disabled on transport failure',async()=>{
 const login=await readFile(new URL('../src/product/Login.tsx',import.meta.url),'utf8');
 const app=await readFile(new URL('../src/product/Workspace.tsx',import.meta.url),'utf8');
 assert.match(login,/\/auth\/start\?rd=/);
 assert.match(login,/disabled=\{!available\|\|checking\}/);
 assert.match(app,/\/auth\/userinfo/);
 assert.match(app,/r\.status===426/);
 const account=await readFile(new URL("../src/product/AccountMenu.tsx",import.meta.url),"utf8");
 assert.match(account,/\/auth\/sign_out/);
 assert.match(account,/Đổi mật khẩu/);
 assert.match(account,/\/profile-api\/avatar/);
 assert.doesNotMatch(login,/type=["']password["']/);
});
test('no preview or guest bypass in product login and workspace',async()=>{
 const login=await readFile(new URL('../src/product/Login.tsx',import.meta.url),'utf8');
 const app=await readFile(new URL('../src/product/Workspace.tsx',import.meta.url),'utf8');
 assert.doesNotMatch(login,/onPreview|nv-login-preview/);
 assert.doesNotMatch(app,/setPreview|nv\.preview|preview=1/);
});
test('account actions stay in Nam Viet branded authenticated flows',async()=>{
 const account=await readFile(new URL('../src/product/AccountMenu.tsx',import.meta.url),'utf8');
 assert.doesNotMatch(account,/\/realms\/namviet\/account/);
 assert.match(account,/\/profile-api\/actions\/profile/);
 assert.match(account,/\/profile-api\/actions\/password/);
});
