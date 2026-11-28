# E17 runtime retrieval diagnostic

Migration 23 grants SELECT only through RLS for PUBLISHED policy clauses and knowledge chunks. The Rust adapter requires REQUESTER inside a scoped transaction, applies dossier kind, effective business date, model digest, lexical priority and pgvector distance. The synthetic API route is under /__diagnostics and is intentionally absent from production OpenAPI. Results are clause citations only; no model output can modify a rule, policy, workflow or dossier state.
