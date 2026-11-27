# E10/E11 runtime slice
E10 exposes current-revision match results and manual confirmation. Only an assigned independent Accountant may confirm a pair of distinct evidence records, and both records must belong to the current dossier revision. The confirmation is immutable, version-fenced, idempotent, audited and outboxed. It does not promote fuzzy similarity to authority; pure Rust exact/tolerance/line matching remains separate from the human confirmation path.

E11 exposes reservation state read-only to the dossier Requester. Reserve/commit/consume/release remain internal transaction ports backed by the existing sorted financial-resource locks and immutable allocation effects. The HTTP layer cannot create or mutate a reservation.
