# E15 — Rule
Scope: Typed DSL và evaluator. Owner A15. Dependencies: E01, E09, E22. Owns `crates/rule-engine/`, `apps/web/src/features/rule-engine/`, `tests/rule-engine/`; migrations assigned centrally.

| ID | Capability | Requirement | Acceptance scenario |
|---|---|---|---|
| E15-R01 | Compiler | schema/type registry, bounded AST, no IO/LLM/code, version | E15-AT01: unknown operator or money currency mismatch |
| E15-R02 | Evaluator | PASS/FAIL/UNKNOWN/ERROR/NOT_APPLICABLE plus severity; tri-state semantics | E15-AT02: unknown under NOT, short circuit evidence |
| E15-R03 | Simulator | run against approved anonymized historical snapshots; compare result distribution | E15-AT03: draft rule not effective in live |
| E15-R04 | Gate | version pinned, mandatory unknown blocks; waiver separate immutable record | E15-AT04: expired waiver and changed snapshot block |

## Interface and integration
Use shared typed IDs, actor scope, Decimal and ports; dependencies expose public interfaces only. Never query another module's private tables directly; coordinated finance locks via E11 transaction port. All mutations emit E22 audit (core port frozen G01); external side effects via E01 outbox. API operations tagged E15; where internal-only no HTTP endpoint required. Error envelope, idempotency, version and evidence conventions from contracts. Implement positive and negative acceptance; actual proof required before DONE.
