//! Pure cross-engine synthetic acceptance smoke; not the durable workflow gate.
use finance_financial::{Constraint, payable};
use finance_integration::{External, Observation, may_release, observe};
use finance_rule_engine::{Expr, Outcome, blocks, evaluate};
use finance_shared::money::{Amount, Currency, Money};
use finance_validation::{Outcome as Validation, invoice_total};
use finance_workflow::{Command, Guards, State, transition};

fn m(n: &str) -> Money {
    Money {
        amount: Amount::parse(n).unwrap(),
        currency: Currency::parse("VND").unwrap(),
    }
}

#[test]
fn payment_ready_requires_verified_total_cap_and_current_snapshot() {
    let check = invoice_total(Some(m("100")), Some(m("10")), Some(m("110")));
    let cap = payable(m("90"), &[Constraint::Applicable(Some(m("100"))), Constraint::Applicable(Some(m("110")))]);
    let rule = evaluate(&Expr::AtMost(Some(m("90")), Some(m("100"))));
    let mut guards = Guards {
        authorized: true,
        current_revision: true,
        complete: true,
        verified: check == Validation::Pass,
        rules_pass: !blocks(rule, true),
        reserved: cap.is_ok(),
        snapshot_current: true,
        ..Default::default()
    };
    assert_eq!(transition(State::Reviewable, Command::Ready, &guards), Some(State::Ready));
    guards.snapshot_current = false;
    assert_eq!(transition(State::Reviewable, Command::Ready, &guards), None);
    assert_eq!(transition(State::Draft, Command::Accept, &guards), None);
}

#[test]
fn advance_missing_coverage_and_settlement_timeout_block() {
    let cap = payable(m("30"), &[Constraint::Applicable(None)]);
    assert!(cap.is_err());
    let unknown = evaluate(&Expr::AtMost(Some(m("30")), None));
    assert_eq!(unknown, Outcome::Unknown);
    assert!(blocks(unknown, true));
    let guards = Guards {
        authorized: true,
        current_revision: true,
        complete: true,
        verified: false,
        rules_pass: !blocks(unknown, true),
        reserved: cap.is_ok(),
        snapshot_current: true,
        ..Default::default()
    };
    assert_eq!(transition(State::Reviewable, Command::Ready, &guards), None);
    let state = observe(External::DispatchPending, Observation::Timeout, false).unwrap();
    assert_eq!(state, External::Unknown);
    assert!(!may_release(state, true));
    assert_eq!(transition(State::AccountingReview, Command::Accept, &guards), None);
}
