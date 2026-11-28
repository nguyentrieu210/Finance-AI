import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

const read=(name)=>readFileSync(new URL('../src/product/'+name,import.meta.url),'utf8');

test('invited staff claims only through the verified same-origin session',()=>{
 const claim=read('InvitationClaim.tsx');
 assert.match(claim,/\/v1\/organizations\/onboarding\/claim/);
 assert.match(claim,/credentials:'same-origin'/);
 assert.match(claim,/Tài khoản chỉ được cấp quyền sau khi được phê duyệt/);
 assert.doesNotMatch(claim,/Bearer\s+|localStorage|sessionStorage/);
});

test('organization admins can issue or revoke invitations and decide requests',()=>{
 const staff=read('StaffAdmin.tsx');
 const decisions=read('OnboardingRequests.tsx');
 assert.match(staff,/onboarding\/invitations\/.*\/issue/);
 assert.match(staff,/delivery_status/);
 assert.match(decisions,/onboarding\/requests/);
 assert.match(decisions,/expected_version:row\.version,approve/);
 assert.match(decisions,/Phê duyệt/);
});
