# E08/E09 current field runtime
The API reads only current-revision fields from the newest OCR run per document. Field identifiers are persistent and separate from the immutable extracted-field primary key. Raw text is derived from bound evidence blocks; normalized values remain CANDIDATE until an independent assigned Accountant records a decision.

Field decisions are typed in Rust before mutation, idempotent, revision/version fenced and append-only. Exact candidate confirmation requires the same bound evidence set; correction requires explicit evidence from the same dossier revision. Every decision increments the dossier version, so older preflight/check results immediately become stale and disappear from current-version reads.
