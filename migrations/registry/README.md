# Migration registry (G01 draft)
A00 allocates globally unique monotonically increasing SQLx migration IDs and records module, dependencies, forward change, rollback/recovery strategy and evidence before merge. No application table or tenant role is created at G00. Do not run development migrations against a production database. Final SQLx path and roles require G01 engineering/security review.
