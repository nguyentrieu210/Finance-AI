#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
use finance_shared::money::Money;

pub mod payment;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Unknown,
    Currency,
    Negative,
    ExceedsCap,
    Overflow,
}

/// Confirmed consumption and outstanding reservation must be disjoint.
pub fn available(
    cap: Money,
    consumed: Money,
    reserved: Money,
    complete: bool,
) -> Result<Money, Error> {
    if !complete {
        return Err(Error::Unknown);
    }
    if cap.currency != consumed.currency || cap.currency != reserved.currency {
        return Err(Error::Currency);
    }
    if [cap, consumed, reserved]
        .iter()
        .any(|m| m.amount.decimal().is_sign_negative())
    {
        return Err(Error::Negative);
    }
    let remaining = cap
        .checked_sub(consumed)
        .and_then(|m| m.checked_sub(reserved))
        .map_err(|_| Error::Overflow)?;
    if remaining.amount.decimal().is_sign_negative() {
        return Err(Error::Negative);
    }
    Ok(remaining)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Constraint {
    Applicable(Option<Money>),
    NotApplicable,
}

pub fn payable(request: Money, caps: &[Constraint]) -> Result<Money, Error> {
    if request.amount.decimal() <= 0.into() {
        return Err(Error::Negative);
    }
    let mut min = None::<Money>;
    for cap in caps {
        match cap {
            Constraint::NotApplicable => (),
            Constraint::Applicable(None) => return Err(Error::Unknown),
            Constraint::Applicable(Some(cap)) => {
                if cap.currency != request.currency {
                    return Err(Error::Currency);
                }
                if cap.amount.decimal().is_sign_negative() {
                    return Err(Error::Negative);
                }
                if min.is_none_or(|m| cap.amount.decimal() < m.amount.decimal()) {
                    min = Some(*cap);
                }
            }
        }
    }
    let limit = min.ok_or(Error::Unknown)?;
    if request.amount.decimal() > limit.amount.decimal() {
        return Err(Error::ExceedsCap);
    }
    Ok(limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_shared::money::{Amount, Currency};
    fn m(n: &str) -> Money {
        Money {
            amount: Amount::parse(n).unwrap(),
            currency: Currency::parse("VND").unwrap(),
        }
    }
    #[test]
    fn cap_never_clamps_or_ignores_missing_history() {
        assert_eq!(available(m("100"), m("30"), m("40"), true), Ok(m("30")));
        assert_eq!(
            available(m("100"), m("70"), m("40"), true),
            Err(Error::Negative)
        );
        assert_eq!(
            available(m("100"), m("0"), m("0"), false),
            Err(Error::Unknown)
        );
        assert_eq!(
            payable(m("60"), &[Constraint::Applicable(Some(m("50")))]),
            Err(Error::ExceedsCap)
        );
        assert_eq!(
            payable(m("10"), &[Constraint::Applicable(None)]),
            Err(Error::Unknown)
        );
    }
}
