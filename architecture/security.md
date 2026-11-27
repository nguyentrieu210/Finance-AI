# Threat model and authorization
| Role | Allowed | Prohibited |
|---|---|---|
| Requester | own/delegated drafts, docs, preflight, handoff | own review/waiver approval, payment mutation |
| Accountant | assigned/company review, critical verification, return/accept | own dossier review, change sealed values |
| Finance approver | scoped exception/policy/reference approval | approve own request or authored policy |
| Policy admin | draft policy/rules/forms/templates/simulation | self publish |
| Org admin | membership/settings within delegated scope | grant self finance authority, implicit access to files |
| Auditor | scoped read/export evidence and events | business mutation |
| Worker | scoped processing commands | approve, waive, transfer money |
| Technical admin | infra/health and break-glass governed separately | unrestricted business read by default |

Every endpoint and object URL checks resource scope, action, actor, assignment, SoD and current permission. RLS defense-in-depth, scoped FK, no BYPASSRLS for app/worker. TLS, S3 private, encryption, short-lived signed URL; revoke/download risk managed via short TTL or proxy. No PII/full bank details/secrets in logs. UI never authority. CSRF protections if cookie auth, CORS allowlist, CSP, rate limits. Provider allowlist/data residency and redaction approved, model no actions.

| Threat | Control | Required proof |
|---|---|---|
| IDOR/cross tenant | scoped authz + RLS + object guard | negative tests API/DB/storage |
| privilege escalation | SoD, grant scope, maker-checker | self approval and revoked role tests |
| malware/zip bomb | AV quarantine, sandbox, resource caps | malicious fixture rejection |
| prompt injection | untrusted content, schema, no tools | adversarial corpus |
| double spend/replay | locks, idempotency, ledger dedupe | concurrent real DB tests |
| forged webhooks | verify signature, timestamp, event dedupe | invalid/replay/out-of-order tests |
| audit tampering | append-only, hash chain, independent checkpoint | tamper/recovery test |
| data exfiltration | provider controls, redaction, egress restrictions | unapproved route fails closed |
| outage/backup loss | restore drill, reconciliation, bounded retries | RPO/RTO recorded |
| CSV/formula injection | escape exports, no macros | adversarial export test |

Security signoff before production; policy/legal retention and incident playbook require owner. Audit integrity detects modification, not proof of identity or absolute immutability against infrastructure administrators.
