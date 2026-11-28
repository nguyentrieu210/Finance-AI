# E03/E04 extension — dynamic permission matrix (implementation draft)

Status: candidate domain implementation; no new live authorization path. Existing
membership, identity lookup and RLS continue to enforce access.

## Model
- Organization membership: verified identity and active company relationship,
  separate from access rights; registration never grants membership.
- Department assignment: many-to-many, exact unit scope and effective interval.
- Job title: descriptive HR data, not authorization.
- Custom role: scoped to company, versioned definition, explicit capability catalog.
- Role assignment: actor, role, exact company/department scope, start/end,
  revocation status. One actor may have many roles and departments.
- Reserved system administration: existing ORG_ADMIN and policy administration
  stay separate; custom financial roles cannot mint administrative privilege.
- Capability catalog: READ_OWN, READ_ASSIGNED, READ_DEPARTMENT, READ_COMPANY,
  WRITE_OWN, REVIEW_ASSIGNED, CONFIRM_DEPARTMENT, APPROVE_ASSIGNED.
- Department scopes never expand into company access. No implicit child or parent
  access. A title or role name does not grant anything by itself.

## Mandatory server enforcement
1. Validate OIDC issuer/subject, verified and active company membership.
2. Resolve resource company, owner, actual department, assignments, workflow
   stage and prior review actors from trusted repositories.
3. Lock active membership, relevant role version, department membership and
   assignments in the same transaction. Use server time, not request time.
4. Resolve effective grants; deny unknown, inactive, expired, ambiguous or absent
   grants. The scoped evaluator does not replace existing legacy guards/RLS.
5. Enforce independent maker, checker, confirmer and final approver. Require
   current workflow prerequisites and approved authority amount outside this
   evaluator; no approval route may use the capability alone.
6. Enforce matching PostgreSQL RLS and guarded write functions. No new permission
   is effective until every applicable read/write path has been mapped and tested.
7. Audit grant changes with actor, target, before/after, reason, version and
   correlation; preserve transactional outbox and idempotency.

## Persistence and API implementation order
- Resolve Migration 62 profile projection, reconcile upstream branch and ledger.
- Add scoped organization membership and immutable role-definition versions;
  foreign keys include tenant/company; effective dates; deny-by-default RLS.
- Add guarded commands to draft, publish, assign and revoke roles; forbid
  self-escalation and alteration of protected administrator privileges.
- Add read-only matrix endpoint and separately guarded mutation endpoints.
- Expose admin matrix with draft/published states and server-returned version.
  Never display a draft as effective permission.
- Integrate each business command/query and prove RLS parity before opening live
  routes. Email invitation and approval remain separate identity workflows.

## Acceptance gates
- Same user can belong to multiple departments and roles; no cross-company
  or cross-department leakage.
- Revoked, expired and future grants deny; inactive identity denies.
- Scope mismatch, missing assignment, duplicate role definitions, workers deny.
- Maker/checker/final approver conflicts deny even with multiple roles.
- Real PostgreSQL tests must cover concurrent revoke, connection-pool reuse,
  cross-tenant FK and RLS, role publication, audit and idempotent retries.
- Browser integration must prove real authenticated admin and unassigned user.

Evidence in this iteration: scoped Rust evaluator and unit tests. Database
persistence, APIs, live policy enforcement and invitation are not yet complete.
