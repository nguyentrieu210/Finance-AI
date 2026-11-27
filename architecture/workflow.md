# Workflow state machines — normative draft
Independent axes: document scan/process, dossier lifecycle, DO approval, payment. State commands explicit; no direct generic status PATCH. Version check + auth + guard + audit/outbox in single DB transaction; external calls outside.

| Command | From → To (dossier) | Actor | Guard / effects |
|---|---|---|---|
| create | none → DRAFT | requester | membership, kind, policy scope |
| process | DRAFT/NEEDS_INPUT → PROCESSING | requester/worker | clean docs, current revision; enqueue |
| process complete | PROCESSING → REVIEWABLE/NEEDS_INPUT | worker | lease fence + revision match; failures explicit |
| edit | DRAFT/NEEDS_INPUT/REVIEWABLE → DRAFT | requester | new revision for material change; stale results invalidated |
| ready | REVIEWABLE → READY | requester | complete checklist, verified inputs, applicable blocking rules PASS or authorized waiver, atomic reserve, snapshot seal |
| export | READY → EXPORTED | worker | rendered bytes + manifest hashes verified; on failure stay READY |
| handoff | EXPORTED → HANDED_OFF | requester | manual evidence or confirmed external receipt; commit reservation when dispatch begins; uncertain attempt stays tracked without fabricated ack |
| begin review | HANDED_OFF → ACCOUNTING_REVIEW | accountant | assigned scope and not owner; approval policy guard when applicable |
| accept | ACCOUNTING_REVIEW → ACCEPTED | accountant | independent critical confirmation, valid snapshot, waivers valid, required DO evidence |
| return | ACCOUNTING_REVIEW → RETURNED | accountant | mandatory reason; disposition of hold reconciled, no silent release |
| reopen | RETURNED → DRAFT | requester | new revision, old snapshot immutable; reconcile old hold before new reserve |
| revise sealed | READY/EXPORTED → DRAFT | requester | no uncertain handoff; authorize release, new revision; old package marked superseded |
| cancel | DRAFT/NEEDS_INPUT/REVIEWABLE/READY/EXPORTED/RETURNED → CANCELLED | authorized actor | no unresolved external dispatch, no paid effect, release allowed remaining only |

HANDED_OFF/ACCEPTED cancel/change via reconciliation workflow, not direct cancellation. No approval or payment implied by ACCEPTED. Document: QUARANTINED→CLEAN/REJECTED; CLEAN→PROCESSING→EXTRACTED/FAILED; verified field decisions independent. DO: NOT_SUBMITTED→DISPATCH_PENDING→ACKNOWLEDGED→APPROVED/REJECTED; timeout UNKNOWN, reconciliation only moves state based on verified observation; manual receipt is not approval. Payment: UNKNOWN/UNPAID→PARTIALLY_PAID/PAID→PARTIALLY_REVERSED/REVERSED on source events, not user toggles. Event order uses source sequence when available; conflicting events enter reconciliation. No inference from stale data.

Waiver lifecycle REQUESTED→APPROVED/REJECTED→EXPIRED/REVOKED; scope result/revision/inputs, reason, expiry and independent approver, policy explicitly allows; non-waivable blockers never overridden. Original result remains unchanged. Re-check authorization, policy revocation, source freshness, approval and waiver validity at ready, handoff and accept; changed inputs require new snapshot or explicit reconciliation, not mutation of sealed record.

Preflight waiver binding: request uses immutable check result ID + revision + input_hash, before ready snapshot exists. On ready, seal references approved waiver IDs and their scope; do not require snapshot ID when requesting waiver. Any change to relevant inputs invalidates the waiver. Critical fields at ready must be verified per policy; independent accountant confirmation is a separate gate at accept.
