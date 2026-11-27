use finance_core::prototype::Error;
use finance_workflow::repository::{Transition, compile_definition};

fn graph() -> Vec<Transition> {
    [
        (
            "PROCESS",
            "DRAFT",
            "PROCESSING",
            "REQUESTER",
            vec!["authorized", "current_revision", "clean_docs"],
        ),
        (
            "PROCESS",
            "NEEDS_INPUT",
            "PROCESSING",
            "REQUESTER",
            vec!["authorized", "current_revision", "clean_docs"],
        ),
        (
            "PROCESS_COMPLETE",
            "PROCESSING",
            "REVIEWABLE",
            "SYSTEM",
            vec![
                "authorized",
                "current_revision",
                "worker_fenced",
                "complete",
            ],
        ),
        (
            "PROCESS_COMPLETE",
            "PROCESSING",
            "NEEDS_INPUT",
            "SYSTEM",
            vec![
                "authorized",
                "current_revision",
                "worker_fenced",
                "incomplete",
            ],
        ),
        (
            "READY",
            "REVIEWABLE",
            "READY",
            "REQUESTER",
            vec![
                "authorized",
                "current_revision",
                "complete",
                "verified",
                "rules_pass",
                "reserved",
                "snapshot_current",
            ],
        ),
        (
            "EXPORT",
            "READY",
            "EXPORTED",
            "SYSTEM",
            vec![
                "authorized",
                "current_revision",
                "artifact_verified",
                "snapshot_current",
            ],
        ),
        (
            "HANDOFF",
            "EXPORTED",
            "HANDED_OFF",
            "REQUESTER",
            vec![
                "authorized",
                "current_revision",
                "external_receipt",
                "reserved",
                "snapshot_current",
                "rules_pass",
            ],
        ),
        (
            "BEGIN_REVIEW",
            "HANDED_OFF",
            "ACCOUNTING_REVIEW",
            "REQUESTER",
            vec!["authorized", "current_revision", "independent_accountant"],
        ),
        (
            "ACCEPT",
            "ACCOUNTING_REVIEW",
            "ACCEPTED",
            "ACCOUNTANT",
            vec![
                "authorized",
                "current_revision",
                "independent_accountant",
                "critical_confirmed",
                "required_external_evidence",
                "snapshot_current",
                "rules_pass",
            ],
        ),
        (
            "RETURN",
            "ACCOUNTING_REVIEW",
            "RETURNED",
            "ACCOUNTANT",
            vec![
                "authorized",
                "current_revision",
                "independent_accountant",
                "return_reason",
                "reconciled",
            ],
        ),
        (
            "REOPEN",
            "RETURNED",
            "DRAFT",
            "REQUESTER",
            vec!["authorized", "current_revision", "reconciled"],
        ),
    ]
    .into_iter()
    .map(|(command, from, to, role, guards)| Transition {
        command: command.into(),
        from: from.into(),
        to: to.into(),
        role: role.into(),
        guard_refs: guards.into_iter().map(str::to_owned).collect(),
    })
    .collect()
}
#[test]
fn complete_graph_is_deterministic() {
    let a = graph();
    let mut b = graph();
    b.reverse();
    for t in &mut b {
        t.guard_refs.reverse();
    }
    assert_eq!(compile_definition(&a, 1), compile_definition(&b, 1));
}
#[test]
fn invalid_graph_fails_closed() {
    let mut missing = graph();
    missing.pop();
    assert!(matches!(
        compile_definition(&missing, 1),
        Err(Error::InvalidInput)
    ));
    let mut skip = graph();
    skip[0].to = "READY".into();
    assert!(matches!(
        compile_definition(&skip, 1),
        Err(Error::InvalidInput)
    ));
    let mut bad = graph();
    bad[0].guard_refs.push("model_says_ok".into());
    assert!(matches!(
        compile_definition(&bad, 1),
        Err(Error::InvalidInput)
    ));
}
