# E19 form lifecycle

Form aggregate identity is stable across immutable version rows. Create/revise are PolicyAdmin-only, idempotent, version-guarded and emit audit/outbox records atomically. The schema compiler validates typed values, conditional required fields, enum options, source paths and 12-column layout. Publication is not a form command: the policy maker/checker transition publishes only the newest draft version per form family and retires older drafts. Requesters can read only the PUBLISHED version through RLS.
