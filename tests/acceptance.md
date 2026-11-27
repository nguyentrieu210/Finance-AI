# Acceptance catalog
All cases planned, not run. See YAML for machine-readable cases.

| ID | Requirement | Scenario | Expected |
|---|---|---|---|
| E01-AT01 | E01-R01 | VND/USD, khác currency, overflow, decimal serialization | Invariant: Decimal chính xác, scale và currency/UOM rõ; reject overflow, không float; assert expected positive behavior and negative rejection where applicable. |
| E01-AT02 | E01-R02 | request thiếu actor, version conflict, clock frozen | Invariant: CommandId, tenant/company, actor, expected version, correlation; UTC clock injectable; assert expected positive behavior and negative rejection where applicable. |
| E01-AT03 | E01-R03 | crash, lease expiry, duplicate delivery | Invariant: Lease có fencing token, heartbeat, retry bounded, dead letter; worker stale không commit; assert expected positive behavior and negative rejection where applicable. |
| E01-AT04 | E01-R04 | rollback không mất event, replay khác payload 409 | Invariant: Transaction scope, idempotency, inbox/outbox atomic; retry conflict bounded; assert expected positive behavior and negative rejection where applicable. |
| E02-AT01 | E02-R01 | token hết hạn, sai issuer, rotated key | Invariant: issuer/audience/algorithm/expiry/JWKS allowlist; fail closed; assert expected positive behavior and negative rejection where applicable. |
| E02-AT02 | E02-R02 | subject không có membership bị từ chối | Invariant: mapping subject nội bộ, phiên và revoke; không tự gia nhập tenant; assert expected positive behavior and negative rejection where applicable. |
| E02-AT03 | E02-R03 | service không gọi approval | Invariant: worker identity scoped, không impersonate approver; assert expected positive behavior and negative rejection where applicable. |
| E02-AT04 | E02-R04 | phiên bị revoke không mutate | Invariant: logout, revoke và refresh theo IdP; không lưu token trong localStorage; assert expected positive behavior and negative rejection where applicable. |
| E03-AT01 | E03-R01 | IDOR, cùng tenant khác company | Invariant: RBAC + ABAC, company, ownership, assigned reviewer, deny by default; assert expected positive behavior and negative rejection where applicable. |
| E03-AT02 | E03-R02 | pool reuse không rò tenant | Invariant: non-owner non-BYPASSRLS, FORCE RLS khi phù hợp, SET LOCAL trong transaction; assert expected positive behavior and negative rejection where applicable. |
| E03-AT03 | E03-R03 | dual-role self approval denied | Invariant: người lập, người yêu cầu ngoại lệ không tự phê duyệt; actor độc lập; assert expected positive behavior and negative rejection where applicable. |
| E03-AT04 | E03-R04 | delegation hết hạn và stale browser | Invariant: quyền có thời hạn, scope và revoke, kiểm tra tại command execution; assert expected positive behavior and negative rejection where applicable. |
| E04-AT01 | E04-R01 | cross-company FK reject | Invariant: tenant/company/department/cost center, unique code và effective dates; assert expected positive behavior and negative rejection where applicable. |
| E04-AT02 | E04-R02 | privilege escalation denied | Invariant: mời/gán/thu hồi role với audit, admin kỹ thuật không mặc định đọc hồ sơ; assert expected positive behavior and negative rejection where applicable. |
| E04-AT03 | E04-R03 | change timezone không sửa lịch sử | Invariant: locale, timezone, currency, retention, provider egress policy versioned; assert expected positive behavior and negative rejection where applicable. |
| E04-AT04 | E04-R04 | disabled org cannot create dossier | Invariant: cost center và company hợp lệ tại ngày nghiệp vụ; assert expected positive behavior and negative rejection where applicable. |
| E05-AT01 | E05-R01 | polyglot, malware, oversized rejected | Invariant: MIME sniff, SHA256, quarantine, AV, quotas và object key scoped; assert expected positive behavior and negative rejection where applicable. |
| E05-AT02 | E05-R02 | seal rồi thay file không sửa bản gốc | Invariant: upload finalize xác nhận checksum; replace tạo ref/revision, không ghi đè; assert expected positive behavior and negative rejection where applicable. |
| E05-AT03 | E05-R03 | viewer mở đúng trang, cross-tenant denied | Invariant: locator page/cell/bbox chuẩn tọa độ, hash original, excerpt; download scoped; assert expected positive behavior and negative rejection where applicable. |
| E05-AT04 | E05-R04 | legal hold không xóa, orphan cleanup | Invariant: classify, supersede, retention hold và delete chỉ theo policy; assert expected positive behavior and negative rejection where applicable. |
| E06-AT01 | E06-R01 | malformed PDF và bomb timeout | Invariant: PDF text layer và raster per page; sandbox, cap CPU/RAM/pages; assert expected positive behavior and negative rejection where applicable. |
| E06-AT02 | E06-R02 | scan nghiêng, chữ dấu, rotated page | Invariant: PaddleOCR isolated; model/version, page bounding boxes, Vietnamese text; assert expected positive behavior and negative rejection where applicable. |
| E06-AT03 | E06-R03 | blur không tự xác nhận | Invariant: điểm chất lượng ảnh và retry preprocessing; low quality human review; assert expected positive behavior and negative rejection where applicable. |
| E06-AT04 | E06-R04 | cache cross-tenant forbidden, model upgrade rerun | Invariant: cache theo tenant/hash/model/config; stale job discard; assert expected positive behavior and negative rejection where applicable. |
| E07-AT01 | E07-R01 | PDF chứa hợp đồng và phụ lục | Invariant: kind candidates, unknown/ambiguous, user confirm; multi-doc split; assert expected positive behavior and negative rejection where applicable. |
| E07-AT02 | E07-R02 | table across pages | Invariant: blocks, reading order, tables, merged cells, continuation pages; assert expected positive behavior and negative rejection where applicable. |
| E07-AT03 | E07-R03 | 1.234 vs 1,234 không đoán | Invariant: giữ raw Unicode, dấu số, ngày, timezone; ambiguity explicit; assert expected positive behavior and negative rejection where applicable. |
| E07-AT04 | E07-R04 | missing table cell stays unknown | Invariant: block IDs link original page; no invented cells; assert expected positive behavior and negative rejection where applicable. |
| E08-AT01 | E08-R01 | unapproved provider blocked | Invariant: approved provider/region, redaction, token cap, timeout, no tools; assert expected positive behavior and negative rejection where applicable. |
| E08-AT02 | E08-R02 | schema violation rejected | Invariant: schema theo document kind, raw+normalized+evidence, model/prompt version; assert expected positive behavior and negative rejection where applicable. |
| E08-AT03 | E08-R03 | high confidence không auto verified | Invariant: candidate, correction, verified separate; critical independent review; assert expected positive behavior and negative rejection where applicable. |
| E08-AT04 | E08-R04 | regression, injection text treated as data | Invariant: golden corpus per field/kind, cost latency and false acceptance; assert expected positive behavior and negative rejection where applicable. |
| E09-AT01 | E09-R01 | invalid date, negative outside signed adjustment | Invariant: required types, enum, date, checksum, decimal scale; assert expected positive behavior and negative rejection where applicable. |
| E09-AT02 | E09-R02 | subtotal mismatch, stale reference | Invariant: document-specific fields and totals, field provenance and freshness; assert expected positive behavior and negative rejection where applicable. |
| E09-AT03 | E09-R03 | missing beneficiary blocks | Invariant: policy lists critical paths and required evidence; unknown not pass; assert expected positive behavior and negative rejection where applicable. |
| E09-AT04 | E09-R04 | late extraction does not overwrite revision | Invariant: dependency graph from document/ref changes to checks/review; assert expected positive behavior and negative rejection where applicable. |
| E10-AT01 | E10-R01 | different issuer same invoice number not same identity | Invariant: exact normalized invoice/vendor/contract identity; fuzzy suggestions only; assert expected positive behavior and negative rejection where applicable. |
| E10-AT02 | E10-R02 | split receipt, incompatible UOM | Invariant: many-to-many allocation, UOM conversion approved, unmatched rows explicit; assert expected positive behavior and negative rejection where applicable. |
| E10-AT03 | E10-R03 | quantity mismatch, rounding boundary | Invariant: line quantity, price, totals, tolerance pinned; no silent rounding; assert expected positive behavior and negative rejection where applicable. |
| E10-AT04 | E10-R04 | ambiguous match remains unknown | Invariant: operands and evidence pairs, manual matching reason and reviewer; assert expected positive behavior and negative rejection where applicable. |
| E11-AT01 | E11-R01 | currency mix, tax rounding, signed adjustment | Invariant: gross/net/tax/discount/retention/FX explicitly typed per policy; assert expected positive behavior and negative rejection where applicable. |
| E11-AT02 | E11-R02 | duplicate paid event, partial payment reversal | Invariant: append-only effects, unique external transaction, reversal link; no double counting; assert expected positive behavior and negative rejection where applicable. |
| E11-AT03 | E11-R03 | two concurrent requests exceed cap; timeout no release | Invariant: atomic lock order, idempotent reserve/commit/consume/release, reconcile uncertainty; assert expected positive behavior and negative rejection where applicable. |
| E11-AT04 | E11-R04 | missing history unknown, negative remaining flagged | Invariant: cap min of applicable verified constraints; no negative balance silently clamped; assert expected positive behavior and negative rejection where applicable. |
| E12-AT01 | E12-R01 | future addendum not used | Invariant: base/addendum effective range, approved adjustments, obligations; assert expected positive behavior and negative rejection where applicable. |
| E12-AT02 | E12-R02 | same goods received twice claim | Invariant: receipt/acceptance lines and billed/paid allocations across dossiers; assert expected positive behavior and negative rejection where applicable. |
| E12-AT03 | E12-R03 | stale or incomplete snapshot unknown | Invariant: immutable verified import, as_of, completeness; no live ERP assumption; assert expected positive behavior and negative rejection where applicable. |
| E12-AT04 | E12-R04 | partial installments, retention release policy | Invariant: per contract/installment balances, retention and advance deduction; assert expected positive behavior and negative rejection where applicable. |
| E13-AT01 | E13-R01 | draft not counted as issued | Invariant: purpose, due date, employee/vendor, amount, verified external issue; assert expected positive behavior and negative rejection where applicable. |
| E13-AT02 | E13-R02 | concurrent settlement against one advance | Invariant: multiple advances, partial application, same currency unless approved FX; assert expected positive behavior and negative rejection where applicable. |
| E13-AT03 | E13-R03 | unverified refund not consumed | Invariant: refund confirmed from source; extra payment separate reservation; assert expected positive behavior and negative rejection where applicable. |
| E13-AT04 | E13-R04 | timezone boundary and extension | Invariant: as-of business date, extensions approved, overdue warning/block policy; assert expected positive behavior and negative rejection where applicable. |
| E14-AT01 | E14-R01 | wrong period or stale data | Invariant: period, account/cost center, original/revised budget version; assert expected positive behavior and negative rejection where applicable. |
| E14-AT02 | E14-R02 | ERP actual already includes local commitment | Invariant: budget reservation and actual reconciled using disjoint buckets; assert expected positive behavior and negative rejection where applicable. |
| E14-AT03 | E14-R03 | multi-resource reservation rollback | Invariant: atomic all-or-none across relevant budgets and contracts; assert expected positive behavior and negative rejection where applicable. |
| E14-AT04 | E14-R04 | nonwaivable cap blocked | Invariant: budget override only if policy permits and authorized; assert expected positive behavior and negative rejection where applicable. |
| E15-AT01 | E15-R01 | unknown operator or money currency mismatch | Invariant: schema/type registry, bounded AST, no IO/LLM/code, version; assert expected positive behavior and negative rejection where applicable. |
| E15-AT02 | E15-R02 | unknown under NOT, short circuit evidence | Invariant: PASS/FAIL/UNKNOWN/ERROR/NOT_APPLICABLE plus severity; tri-state semantics; assert expected positive behavior and negative rejection where applicable. |
| E15-AT03 | E15-R03 | draft rule not effective in live | Invariant: run against approved anonymized historical snapshots; compare result distribution; assert expected positive behavior and negative rejection where applicable. |
| E15-AT04 | E15-R04 | expired waiver and changed snapshot block | Invariant: version pinned, mandatory unknown blocks; waiver separate immutable record; assert expected positive behavior and negative rejection where applicable. |
| E16-AT01 | E16-R01 | overlap active scopes rejected | Invariant: draft/review/publish/retire/revoke; maker-checker; immutable versions; assert expected positive behavior and negative rejection where applicable. |
| E16-AT02 | E16-R02 | unmapped rule cannot publish | Invariant: rule/checklist/form/workflow links to approved source clause; assert expected positive behavior and negative rejection where applicable. |
| E16-AT03 | E16-R03 | ambiguous effective version | Invariant: company + kind + effective business date, priority explicit; conflict blocks; assert expected positive behavior and negative rejection where applicable. |
| E16-AT04 | E16-R04 | revoked version blocks new submission | Invariant: diff, affected dossiers, re-evaluate open revisions on required change; assert expected positive behavior and negative rejection where applicable. |
| E17-AT01 | E17-R01 | draft or revoked clause not retrieved for current advice | Invariant: approved versioned clauses only, scoped chunk and embedding; assert expected positive behavior and negative rejection where applicable. |
| E17-AT02 | E17-R02 | cross-tenant retrieval forbidden | Invariant: tenant/company/policy version filters, lexical+vector search; assert expected positive behavior and negative rejection where applicable. |
| E17-AT03 | E17-R03 | hallucinated citation not shown as verified | Invariant: cite clause and evidence, distinguish guidance from decisions; assert expected positive behavior and negative rejection where applicable. |
| E17-AT04 | E17-R04 | prompt injection cannot publish | Invariant: AI proposes diff or rule draft; no autonomous publish; assert expected positive behavior and negative rejection where applicable. |
| E18-AT01 | E18-R01 | direct status PATCH denied | Invariant: revision, state transitions and optimistic locking, ready seal; assert expected positive behavior and negative rejection where applicable. |
| E18-AT02 | E18-R02 | stale check run or failed reservation blocks | Invariant: checklist, rule results, reservations, evidence and snapshot atomic; assert expected positive behavior and negative rejection where applicable. |
| E18-AT03 | E18-R03 | self review or missing confirmation denied | Invariant: assign queue, critical confirmation, accept/return, SoD; assert expected positive behavior and negative rejection where applicable. |
| E18-AT04 | E18-R04 | unknown DO cannot release reservations | Invariant: waiver approval, expiry, manual handoff, unknown external status handling; assert expected positive behavior and negative rejection where applicable. |
| E19-AT01 | E19-R01 | invalid field dependency cycle | Invariant: typed fields, condition, source path, required and display layout; assert expected positive behavior and negative rejection where applicable. |
| E19-AT02 | E19-R02 | hidden required policy inconsistency rejected | Invariant: server validates field schema, dynamic visibility and required fields; assert expected positive behavior and negative rejection where applicable. |
| E19-AT03 | E19-R03 | old dossier opens correct form | Invariant: published form pinned to revision, migration preview only; assert expected positive behavior and negative rejection where applicable. |
| E19-AT04 | E19-R04 | draft not used on active dossier | Invariant: admin preview, draft save, maker-checker publish via policy; assert expected positive behavior and negative rejection where applicable. |
| E20-AT01 | E20-R01 | malicious DOCX and unmapped placeholder rejected | Invariant: version, mapping, sandbox, macros forbidden, approved fonts; assert expected positive behavior and negative rejection where applicable. |
| E20-AT02 | E20-R02 | Vietnamese fonts, multi-page table, conversion fail | Invariant: Decimal formatting, tables, pagination, deterministic input manifest; assert expected positive behavior and negative rejection where applicable. |
| E20-AT03 | E20-R03 | same idempotency key returns same package | Invariant: ordered PDF + originals index, evidence manifest, hashes, signed access; assert expected positive behavior and negative rejection where applicable. |
| E20-AT04 | E20-R04 | storage failure no EXPORTED | Invariant: atomic success only after storage verify; retry cleanup and stale protection; assert expected positive behavior and negative rejection where applicable. |
| E21-AT01 | E21-R01 | LLM signal cannot auto approve or reject | Invariant: duplicate/near match, vendor bank changes, anomalous amounts; advisory; assert expected positive behavior and negative rejection where applicable. |
| E21-AT02 | E21-R02 | unauthorized closure rejected | Invariant: assign and resolve signal with reason, evidence, owner; assert expected positive behavior and negative rejection where applicable. |
| E21-AT03 | E21-R03 | changed revision invalidates waiver | Invariant: link immutable finding and approval, expiry/revocation; no override nonwaivable; assert expected positive behavior and negative rejection where applicable. |
| E21-AT04 | E21-R04 | unresolved blocking signal prevents ready | Invariant: severity and routing by approved policy, no automatic transfer; assert expected positive behavior and negative rejection where applicable. |
| E22-AT01 | E22-R01 | rollback leaves neither effect nor event | Invariant: mutation event same transaction, actor, scope, correlation, revision, redacted diff; assert expected positive behavior and negative rejection where applicable. |
| E22-AT02 | E22-R02 | tamper detection and delete denied | Invariant: append-only role, per-tenant sequence/hash chain, external checkpoint optional required prod; assert expected positive behavior and negative rejection where applicable. |
| E22-AT03 | E22-R03 | auditor cannot read other company | Invariant: scoped auditor query and export, access event, no raw secret; assert expected positive behavior and negative rejection where applicable. |
| E22-AT04 | E22-R04 | restore integrity and retention conflict | Invariant: legal hold and approved deletion with tombstone, backup restore verify chain; assert expected positive behavior and negative rejection where applicable. |
| E23-AT01 | E23-R01 | manual submitted not approved | Invariant: reference import verified by maker-checker; manual handoff with receipt; assert expected positive behavior and negative rejection where applicable. |
| E23-AT02 | E23-R02 | timeout unknown, duplicate/out-of-order callback | Invariant: capability discovery, external idempotency, status mapping, signed webhook or polling; assert expected positive behavior and negative rejection where applicable. |
| E23-AT03 | E23-R03 | duplicate transaction, stale import | Invariant: read-only snapshots and settlement/payment events, freshness/completeness; assert expected positive behavior and negative rejection where applicable. |
| E23-AT04 | E23-R04 | lost ack and reversal, no auto-release | Invariant: unknown external state queue, authoritative lookup, authorized correction; no transfer endpoint; assert expected positive behavior and negative rejection where applicable. |
| E24-AT01 | E24-R01 | returned/reopened case counted once | Invariant: first-pass accepted, active time, rule coverage denominator and excluded unknown; assert expected positive behavior and negative rejection where applicable. |
| E24-AT02 | E24-R02 | no cross-company aggregate leak | Invariant: tenant/company filters, time zone, pagination, aggregates; assert expected positive behavior and negative rejection where applicable. |
| E24-AT03 | E24-R03 | dead-letter alarm and unknown state | Invariant: queue age, errors, cost, stale references and reservations; assert expected positive behavior and negative rejection where applicable. |
| E24-AT04 | E24-R04 | CSV formula injection and unauthorized export | Invariant: authorized report, masked PII, audit and reproducible filter; assert expected positive behavior and negative rejection where applicable. |
| SYS-AT01 | SYSTEM | Two parallel reservations 60/60 against cap 100 | Exactly one succeeds; remaining 40; one active hold; no orphan audit. |
| SYS-AT02 | SYSTEM | Three dossier types end-to-end | Payment, advance, settlement each sealed, exported, manually handed off and independently reviewed with evidence. |
| SYS-AT03 | SYSTEM | Provider timeout or malicious instructions | No PASS or approval; failed job visible, retry bounded. |
| SYS-AT04 | SYSTEM | DO lost acknowledgement | UNKNOWN and hold remains; reconciliation does not duplicate submission. |
| SYS-AT05 | SYSTEM | ERP duplicate and partial reversal | Unique effects, reversal bounded; source status reconciled. |
| SYS-AT06 | SYSTEM | Policy revoke after seal | Old snapshot unchanged; new handoff blocked pending approved resolution. |
| SYS-AT07 | SYSTEM | Restore DB and object storage | Manifest hashes and audit checkpoint verify; measure RPO/RTO. |
| SYS-AT08 | SYSTEM | Cross-tenant direct and indirect access | API/DB/object/RAG/report deny, no data leakage. |
| SYS-AT09 | SYSTEM | Approved corpus evaluation | Report per-field accuracy, false PASS blocker corpus, evidence accuracy, latency and cost; target gates reviewed. |
| SYS-AT10 | SYSTEM | Concurrent revise and export | Old output not active; stale worker cannot change new revision. |
| SYS-AT11 | SYSTEM | Partial settlement plus confirmed refund | No overlap; extra request separately controlled. |
| SYS-AT12 | SYSTEM | Same idempotency key with different payload | 409 and no mutation. |
| SYS-AT13 | SYSTEM | Role revocation mid-session | Next command denied; audit records attempted action. |
| SYS-AT14 | SYSTEM | Waiver expires before accept | Accept blocked; original finding remains. |
