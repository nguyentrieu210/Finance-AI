# Staff administration candidate — migration 64
Five scoped admin resources at /v1/organizations/staff/{kind}: invitations,
positions, departments, roles, assignments. GET accepts after UUID and limit
1–100. POST accepts a client-generated nonnil ID, optional expected_version,
a resource-specific payload and an Idempotency-Key. Null expected_version creates
a new resource; an explicit version changes an existing one. No API accepts
tenant/company/actor from JSON; the existing authenticated scope determines them.
Database commands are restricted to active ORG_ADMIN membership and company scope.
Each successful mutation records the request key, command/correlation IDs,
payload and resulting version in an append-only staff_admin_change ledger.
Roles have immutable capability revisions; assignment references an exact
revision. No implicit hierarchy or title grants. Existing financial controls
remain authoritative. When a financial adapter is implemented, only active
current role revisions, current membership, valid assignments and active
department mappings may participate. It must lock and revalidate them in the
same transaction as the dossier action. Do not activate an old revision
automatically. Revoking a role assignment cannot revoke the protected legacy
membership. The invitation resource only creates/cancels drafts, not sends.
Actual email delivery, registration matching, approval, provisioning and
membership lifecycle integration remain release gates.
The optional staff configuration tab is hidden by default. Enable
VITE_STAFF_ADMIN_ENABLED=true only after backend, migration and authorized
browser verification. Existing member/department pages stay usable.

Migration 64 was concurrently applied to the live database before the isolated test finished. Its exact original checksum is preserved; migration 65 corrects the ambiguous department kind reference. Apply 65 before enabling the staff module.
