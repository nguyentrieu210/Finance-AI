#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
use finance_shared::money::Money;

pub mod compiler;
pub mod repository;

pub mod simulator;
pub mod runtime;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Pass,
    Fail,
    Unknown,
    Error,
    NotApplicable,
}

pub enum Expr {
    Known(Option<bool>),
    AtMost(Option<Money>, Option<Money>),
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

/// Bounded deterministic subset; all children evaluated so errors propagate.
pub fn evaluate(expr: &Expr) -> Outcome {
    fn inner(expr: &Expr, depth: usize, nodes: &mut usize) -> Outcome {
        use Outcome::*;
        *nodes += 1;
        if depth > 16 || *nodes > 256 {
            return Error;
        }
        match expr {
            Expr::Known(Some(true)) => Pass,
            Expr::Known(Some(false)) => Fail,
            Expr::Known(None) => Unknown,
            Expr::AtMost(Some(a), Some(b)) if a.currency == b.currency => {
                if a.amount.decimal() <= b.amount.decimal() {
                    Pass
                } else {
                    Fail
                }
            }
            Expr::AtMost(Some(_), Some(_)) => Error,
            Expr::AtMost(_, _) => Unknown,
            Expr::Not(e) => match inner(e, depth + 1, nodes) {
                Pass => Fail,
                Fail => Pass,
                other => other,
            },
            Expr::And(items) | Expr::Or(items) => {
                if items.is_empty() || items.len() > 256 {
                    return Error;
                }
                let mut pass = false;
                let mut fail = false;
                let mut unknown = false;
                for item in items {
                    match inner(item, depth + 1, nodes) {
                        Error => return Error,
                        Pass => pass = true,
                        Fail => fail = true,
                        _ => unknown = true,
                    }
                }
                if matches!(expr, Expr::And(_)) {
                    if fail {
                        Fail
                    } else if unknown {
                        Unknown
                    } else {
                        Pass
                    }
                } else if pass {
                    Pass
                } else if unknown {
                    Unknown
                } else {
                    Fail
                }
            }
        }
    }
    inner(expr, 1, &mut 0)
}

pub fn applicable(applicability: Option<bool>, expr: &Expr) -> Outcome {
    match applicability {
        Some(false) => Outcome::NotApplicable,
        None => Outcome::Unknown,
        Some(true) => evaluate(expr),
    }
}

pub fn blocks(outcome: Outcome, mandatory: bool) -> bool {
    mandatory && !matches!(outcome, Outcome::Pass | Outcome::NotApplicable)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_and_error_propagation() {
        assert_eq!(
            evaluate(&Expr::Not(Box::new(Expr::Known(None)))),
            Outcome::Unknown
        );
        assert_eq!(
            evaluate(&Expr::And(vec![
                Expr::Known(None),
                Expr::Known(Some(false))
            ])),
            Outcome::Fail
        );
        assert_eq!(
            evaluate(&Expr::Or(vec![Expr::Known(Some(true)), Expr::And(vec![])])),
            Outcome::Error
        );
        assert!(blocks(Outcome::Unknown, true));
        assert!(!blocks(Outcome::NotApplicable, true));
    }
}
