# Financial rule contract
Example policy only; no business approval. All rule IDs map to source clause before publish.

## Typed DSL
JSON AST: `{"op":"lte","args":[{"ref":"payment.requested"},{"ref":"contract.available"}]}`. Allowlisted refs and types (money with currency, decimal quantity/unit, date, string, boolean, enum, set). Operators eq/ne/lt/lte/gt/gte/and/or/not/exists/in/sum; arithmetic only compatible types; date/time fixed from snapshot; no arbitrary code, IO, loops or model calls. Bound AST depth/size, collection sizes and evaluation time. Compile schema/type check, fixture test, then publish maker-checker. Missing, ambiguous or stale input => UNKNOWN; malformed runtime => ERROR. NOT_APPLICABLE only when explicit applicability known false. Mandatory unknown/error/fail blocks subject to normative v2 gate; warning requires acknowledgement when configured. Waiver retains original result.

| ID | Applicable | Criterion / evidence | Default draft severity |
|---|---|---|---|
| R01 | PAYMENT | contract number agrees across docs | BLOCK |
| R02 | PAYMENT | requested amount <= verified remaining contract balance | BLOCK |
| R03 | PAYMENT | approved PO qty vs accepted qty, correct UOM | BLOCK |
| R04 | PAYMENT | unit price vs valid contract/addendum | BLOCK |
| R05 | PAYMENT | required acceptance record present and verified | BLOCK |
| R06 | PAYMENT | beneficiary vs verified vendor master | BLOCK, no waiver |
| R07 | ALL | authority and signature confirmation per policy | BLOCK; human verification |
| R08 | PAYMENT | invoice identity and allocation not duplicate | BLOCK, no waiver |
| R09 | ALL | budget availability if applicable | BLOCK / policy |
| R10 | PAYMENT | tax and totals reconcile under rounding policy | BLOCK |
| R11 | ADVANCE | purpose, amount, due date, approved request | BLOCK |
| R12 | ADVANCE | outstanding advance policy | BLOCK / policy |
| R13 | SETTLEMENT | link to verified advance and valid outstanding balance | BLOCK |
| R14 | SETTLEMENT | sum eligible costs and refunds, extra amount reconciled | BLOCK |
| R15 | SETTLEMENT | overdue settlement according to policy | WARNING / policy |
| R16 | ALL | all required doc types / mandatory fields verified | BLOCK |
| R17 | ALL | currency consistency or verified FX rule | BLOCK |
| R18 | ALL | unusual narrative or fuzzy mismatch | WARNING, never auto-fail based only on LLM |
| R19 | ALL | reference freshness / completeness | BLOCK if required |
| R20 | ALL | current policy version and snapshot hash valid | BLOCK, no waiver |

| R21 | PAYMENT | Verified obligation due and accepted value cap | BLOCK |
| R22 | PAYMENT | Partial invoice allocations cumulative cap | BLOCK |
| R23 | PAYMENT | Addendum effective date and authority | BLOCK |
| R24 | PAYMENT | Retention release has independent obligation | BLOCK |
| R25 | PAYMENT | Advance deduction not counted twice | BLOCK |
| R26 | PAYMENT | Invoice adjustment references original | BLOCK |
| R27 | PAYMENT | Receipt line allocation not reused | BLOCK |
| R28 | PAYMENT | Discount and tax rounding policy reconcile | BLOCK |
| R29 | ALL | Active reservation plus confirmed consumption <= cap | BLOCK |
| R30 | ALL | Verified FX source and date where applicable | BLOCK |
| R31 | ALL | Source coverage complete through cutoff | BLOCK |
| R32 | ALL | Source snapshot watermark excludes overlap | BLOCK |
| R33 | ADVANCE | Advance amount within approved purpose cap | BLOCK |
| R34 | ADVANCE | Existing unresolved advances within policy | BLOCK / policy |
| R35 | SETTLEMENT | Partial allocation <= verified advance remaining | BLOCK |
| R36 | SETTLEMENT | Confirmed refund source, no double settlement | BLOCK |
| R37 | SETTLEMENT | Extra expense separate approved payment | BLOCK |
| R38 | SETTLEMENT | Extension approval covers due date | WARNING / policy |
| R39 | ALL | Bank account change independently confirmed | BLOCK, no waiver |
| R40 | ALL | Actor separation and delegation effective | BLOCK, no waiver |
| R41 | ALL | Waiver current and scope matches result | BLOCK |
| R42 | ALL | Document hash matches evidence | BLOCK, no waiver |
| R43 | ALL | No unresolved external dispatch before release | BLOCK |
| R44 | ALL | Mandatory critical fields independently confirmed at accept | BLOCK |

Output: rule_id/version, input_hash, outcome, severity, operands as typed values, source evidence IDs, reason code, explanation, evaluation time/engine version. Do not include unmasked bank accounts in logs. RAG explanations must use approved policy references; explanations do not affect outcome.

## V2 semantics
See architecture/rules-policy.md, financial.md and rule-dsl.schema.json. Catalog is illustrative only; no production activation before policy approval.
