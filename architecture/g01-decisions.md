# G01 decision register — technical prototype only

No item below is an owner signoff. Continue isolated synthetic-data technical work only within approved scope; all production, policy activation and real data processing fail closed.

| ID | Required decision | Owner | Prototype default / limitation | State |
|---|---|---|---|---|
| D01 | Signed policies, required dossier checklist, clauses and non-waivable blockers | Finance | No published business policy; examples draft only | OPEN |
| D02 | Contract balances, taxes, retention, advances, invoice adjustments, rounding and FX | Finance | Exact decimals; no default tax/FX/rounding inference | OPEN |
| D03 | Form templates and mandatory fields | Finance | Synthetic fixtures only | OPEN |
| D04 | Source ownership, completeness, watermark and freshness | Finance + integration | Missing coverage UNKNOWN; no assumed payments | OPEN |
| D05 | Review order, approval authority and exception delegation | Finance + security | No self approval; no actual authorization grants inferred | OPEN |
| D06 | Identity issuer, audiences, role mapping and worker trust | Security | Deny by default; no production identity provider | OPEN |
| D07 | Object storage, quota, grant TTL, malware, egress and key handling | Security + engineering | Proposed bounded staging/generation contract; verify actual provider | OPEN |
| D08 | Retention, legal hold, deletion and audit checkpoint | Security + finance | No destructive production cleanup | OPEN |
| D09 | AI provider, residency, minimization and benchmark corpus | Security + finance | AI egress disabled; synthetic data only | OPEN |
| D10 | DO/ERP sandbox, contracts and reconciliation | Integration + finance | Manual handoff design only; live adapters gated G05 | OPEN |
| D11 | SLO, scale, backup and restore objectives | Engineering + finance | Spec targets are provisional | OPEN |
| D12 | OpenAPI, table constraints, shared Rust ports, migration ordering | Engineering | Review technical draft.2, planned ownership registry; full DDL remains unimplemented | OPEN |

## Evidence and boundary
G00 and previous G01 CI success are engineering test evidence only. G01 is not DONE; no downstream task is marked approved. Owner records must include decision ID, approved revision, signer role, date, evidence and applicability. Approval of a technical prototype does not approve live payment, policies, credentials, deployments or actual financial data.
