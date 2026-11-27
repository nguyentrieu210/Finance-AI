# E20 queued export and fenced worker completion
POST /v1/dossiers/{dossier_id}/exports now accepts only the current READY sealed snapshot and its published template, creates one QUEUED package and an EXPORT_REQUESTED outbox event, and leaves the dossier version unchanged while rendering is pending. The package cannot be marked READY by the API.

The worker path reuses finance_templates::verified_manifest, the same guard exercised by the isolated real DOCX/XLSX/LibreOffice PDF probe. A finance_job_worker lease/fence is required. The PostgreSQL completion function atomically verifies the active lease, marks the package READY with a deterministic manifest hash/artifact count/bytes, moves the dossier READY -> EXPORTED, completes job_attempt/job/inbox, and records EXPORT_COMPLETED audit. Stale leases or changed dossier versions fail closed.


## Runtime worker boundary
`finance-worker` now materializes and claims only `EXPORT_REQUESTED` events, maintains the database lease with heartbeats, and never acknowledges unrelated outbox events. The renderer/storage integration is an explicit command adapter. The child process receives only tenant/company/job/fence plus an optional non-secret config path and storage endpoint; the worker database DSN and parent environment are not inherited. The adapter must return a bounded `finance.export.receipt.v1` JSON receipt containing expected and storage-readback SHA-256 values. Rust rejects malformed, duplicate, mismatched or stale receipts before invoking the fenced atomic completion transaction. Exit 75 is retryable; other adapter failures are permanent and flow through the bounded dead-letter policy.

The restricted PostgreSQL integration test executes the actual worker binary with a synthetic adapter receipt; the separate isolated office-export gate generates and rereads real DOCX/XLSX/PDF bytes. A production storage/renderer adapter and workload identity configuration remain deployment inputs, not embedded credentials.
