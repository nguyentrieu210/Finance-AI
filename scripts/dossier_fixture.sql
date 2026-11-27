-- Run after earlier revocation tests; restore synthetic requester only in disposable DB.
UPDATE finance.membership SET active=true, valid_until=NULL WHERE tenant_id='00000000-0000-0000-0000-000000000001' AND company_id='00000000-0000-0000-0000-00000000000a' AND actor_id='00000000-0000-0000-0000-0000000003e9' AND role='REQUESTER';
INSERT INTO finance.identity_subject(tenant_id,id,issuer,subject) VALUES ('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-0000000003ec','https://synthetic.invalid','second-requester');
INSERT INTO finance.membership(tenant_id,company_id,actor_id,role) VALUES ('00000000-0000-0000-0000-000000000001','00000000-0000-0000-0000-00000000000a','00000000-0000-0000-0000-0000000003ec','REQUESTER');
