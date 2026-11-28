# E06/E08 synthetic invoice worker candidate

Opt-in worker configuration invokes a local Tesseract executable on bounded, hash-verified PNG bytes only after the restricted database read port confirms a CLEAN synthetic invoice, current dossier revision and active job lease. The worker maintains the lease, parses TSV geometry and extracts six invoice candidate fields. The completion transaction rechecks lease, generation, source digest and the current dossier under lock, writes immutable OCR/extraction evidence and candidate fields, and commits job/inbox together. Nothing is auto-verified or approved.

Migration 20261128031100 remains a review candidate. This path does not implement real-user upload, production object storage, PDF/multi-page OCR, external OCR providers, other dossier document types, separate classify/extract jobs, or live AI egress. No approval or CI evidence is inferred.
