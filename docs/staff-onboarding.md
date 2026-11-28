# Migration 66 candidate
A draft invitation can be issued for seven days. The prospective employee
authenticates with Keycloak and submits their own verified email claim. No
request body can choose issuer, subject or email. The claim creates a pending
request only, not a membership. An existing organization administrator approves
or rejects the request using its expected version. Approval explicitly creates
a requester membership; there is no implicit administrator or approver grant.
Email delivery is not yet configured in this workflow. Issuance is not a
delivery receipt. A separate delivery adapter and outbox are needed.
The effective-grants endpoint returns a preview for the authenticated actor.
It filters active role revisions, current assignments, active membership and
department mappings. It does not yet authorize financial resources. Financial
RLS and existing guards remain unchanged pending explicit dossier department
and reviewer facts, lock integration, and end-to-end tests.
Migration 66 was tested in disposable PostgreSQL with synthetic data and must
not be considered live until its ledger and new binary are verified.
