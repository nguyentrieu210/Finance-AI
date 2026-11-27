# Data Model
All tenant-owned records have tenant_id, company_id where applicable; composite FK/unique include tenant scope. UUID IDs, UTC timestamps. Monetary values NUMERIC(28,8) and ISO currency; enforce currency-specific rounding at domain boundary; quantities Decimal, units explicit. JSONB only extensible typed payloads; finance totals normalized. No float. Actor metadata from auth context.

| Entity | Key fields / relations | Constraints |
|---|---|---|
| tenant / company | id, code, locale, policy timezone | tenant code unique; company scoped |
| membership / permission | subject, tenant, company, roles, effective range | deny by default; revoke checks |
| dossier | id, code, kind, owner_id, status, current_revision, lock_version | kind PAYMENT/ADVANCE/SETTLEMENT; unique code |
| dossier_revision | dossier_id, number, policy_version_id, input_hash, sealed_at | immutable when sealed; unique number |
| document_blob | hash, storage_key, MIME, bytes, scan_state | private, immutable; quarantine until clean |
| document_ref | revision_id, blob_id, kind, supersedes | FK to scoped blob; new revision on change after seal |
| extraction_run | doc_ref, provider/model/schema_version, status, latency | input fingerprint, retry key |
| extracted_field | run_id, path, raw, normalized, type, confidence, locator | evidence immutable; confidence non-financial |
| field_decision | field_id, revision_id, actor, value, evidence_id, reason | append-only correction/confirmation |
| evidence | doc_ref, page/cell/bbox, excerpt, hash, locator_version | references original; source type document/reference/policy |
| vendor_ref / contract_ref / invoice_ref | external key, version, currency, effective, source_snapshot | verified freshness, normalized identity |
| contract_line / invoice_line | parent_id, item_key, qty, uom, unit_price, amount, tax | nonnegative by type or explicitly signed adjustments |
| reference_snapshot | source, as_of, verified_by, checksum, imported_at | immutable; stale threshold policy |
| advance_ref / settlement_link | advance key, issued amount, applied amount, refund, extra, settlement dossier | same currency; balance invariant; concurrency guards |
| payment_ref | external txn, contract/invoice, amount, currency, state | unique external key; reconciliation status |
| budget_ref | period, cost center, approved, committed, actual, source | snapshot provenance; applicable policy |
| reservation | dossier_id, revision, source key, amount, currency, status, idempotency key | active normalized invoice identity exclusive where policy requires; lock financial balance; partial invoices allocation-aware |
| policy_version | policy id, version, status, effective_from/to, owner, approver | immutable published; no conflicting active range per scope |
| rule_version | policy_version, rule_id, typed_ast, severity, override flag, clause | unique rule in version, typed validated |
| checklist_item | policy_version, kind, condition AST, requirement, template | typed applicability |
| template_version | id, kind, object key/hash, field mapping, version, status | publish with fixture render |
| check_run | revision_id, policy_version, input_hash, engine_version, status | stale detection; one canonical result per fingerprint |
| check_result | run_id, rule_version, outcome, severity, operands, evidence, reason | PASS/FAIL/UNKNOWN/ERROR/NOT_APPLICABLE; no mutable override |
| waiver | check_result, revision, input_hash, requester, approver, reason, expiry, state | SoD; rule must allow; scope pinned |
| review_decision | snapshot_id, actor, type, critical confirmations, reason | append-only; no self review |
| package | snapshot_id, manifest_hash, blob_id, status | unique export idempotency; immutable success |
| handoff | package_id, channel, external_id, state, evidence | manual/API distinct; no inferred ack |
| external_status | handoff/payment key, external event id, state, observed_at, source | idempotent and ordered; stale status flagged |
| audit_event | event id, actor, scope, resource, action, diff, correlation, time | append-only app role; redact PII; restricted reads |
| job / outbox / inbox | type, payload refs, lease, attempt, status, dedupe key | unique dedupe, visibility and recovery |
| idempotency_record | actor, route, key, request hash, result ref | conflict if different input; expiry policy |


## V2 additional tables / keys
| Entity | Fields and constraints |
|---|---|
| dossier_snapshot | id, tenant, company, dossier, revision, policy/form/template/workflow versions, check_run, input_hash, reference_cutoff, sealed_at, canonical_manifest_hash; unique revision+hash; immutable; review/package FK scoped |
| snapshot_member | snapshot_id, kind, resource_version_id, digest; immutable; all inputs including field decisions, FX and approved tolerances |
| financial_resource | id, kind CONTRACT/INVOICE/ADVANCE/BUDGET/OBLIGATION, source_ref, currency, version; unique scoped identity; locking anchor |
| allocation_effect | id, resource_id, dossier_snapshot_id, external_event_id, effect_kind, amount, currency, reversal_of, occurred_at; immutable; external id dedupe; partial reversal sum <= original |
| reservation_line | reservation_id, resource_id, amount, currency, state, version; unique reservation+resource; positive amount; lock resource before changing; partial consumption represented by separate effects and remaining amount |
| source_coverage | reference_snapshot_id, resource_type, scope, complete_through, imported_by, verified_by, state; maker-checker; missing coverage UNKNOWN |
| policy_clause | version, source_doc, locator, content_hash, clause_code; approved source |
| form_version / workflow_version | policy_scope, schema/state table, version, lifecycle, hash, approval; immutable published |
| rule_simulation | draft rule version, corpus version, engine version, aggregated diff, created_by; no production mutation |
| risk_signal / risk_decision | snapshot, code, evidence, severity, assigned actor, resolution, timestamp; append decision |
| review_assignment / critical_confirmation | snapshot, reviewer, field_path, verified evidence id, decision version; SoD |
| external_attempt | handoff id, channel, command key, request hash, external ref, state, last_error_code; uncertain attempts retained |
| audit_checkpoint | tenant, sequence, digest, timestamp, independent storage reference |
| job_attempt | job, fence_token, attempt, started_at, lease_until, terminal status; stale fences cannot commit |

## Constraints to implement and test
Composite tenant/company FKs; where tenant-global registry allowed, explicit scope rule. RLS with non-owner application and worker roles, transaction-local scope; migration admin separate. NUMERIC(28,8) at rest, checked Decimal conversion, rounding boundary in domain. Snapshot hash canonical field ordering and explicit versions, never mutable live pointers. Check results immutable; outcome separate severity; waivers independent. All financial resource locks acquired in deterministic sorted order inside one DB transaction; no external HTTP in transaction. Audit/outbox and domain writes atomic. See architecture/financial.md and workflow.md for normative invariants. Schema is logical; SQL DDL and migration tests are G01 deliverables, not included as executable migrations.
