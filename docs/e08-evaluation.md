# E08 extraction evaluation gate

The evaluator in `finance_extraction::evaluation` is deterministic and provider-neutral. It requires corpus coverage for every supported document-kind/field pair and reports candidate-level false acceptance, misses, p95 latency, token counts and integer cost units.

A **candidate is never a verified field or a financial approval**. Prompt-like text inside a document remains document data and the extraction result still requires human review.

CI currently runs a synthetic golden corpus and adversarial prompt-text regression through the normal Rust workspace tests. This proves the evaluator and fail-closed thresholds, not production model quality. A representative labeled business corpus, approved provider/region and real provider cost/latency report remain release inputs for the pilot/production signoff.
