#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
use finance_shared::money::Money;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Pass,
    Fail,
    Unknown,
    Error,
}

/// Missing or unverified operands never pass. Inputs must be from a scoped
/// verified snapshot; this pure function cannot attest their provenance.
pub fn invoice_total(net: Option<Money>, tax: Option<Money>, gross: Option<Money>) -> Outcome {
    let (Some(net), Some(tax), Some(gross)) = (net, tax, gross) else {
        return Outcome::Unknown;
    };
    if [net, tax, gross]
        .iter()
        .any(|m| m.amount.decimal().is_sign_negative())
    {
        return Outcome::Fail;
    }
    match net.checked_add(tax) {
        Ok(value) if value == gross => Outcome::Pass,
        Ok(value) if value.currency == gross.currency => Outcome::Fail,
        _ => Outcome::Error,
    }
}

pub fn critical_fields(required: &[bool], source_current: bool) -> Outcome {
    if required.is_empty() || !source_current || required.iter().any(|v| !v) {
        Outcome::Unknown
    } else {
        Outcome::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_shared::money::{Amount, Currency};

    fn money(n: &str) -> Money {
        Money {
            amount: Amount::parse(n).unwrap(),
            currency: Currency::parse("VND").unwrap(),
        }
    }

    #[test]
    fn invoice_and_missing_evidence() {
        assert_eq!(
            invoice_total(Some(money("100")), Some(money("10")), Some(money("110"))),
            Outcome::Pass
        );
        assert_eq!(
            invoice_total(Some(money("100")), Some(money("10")), Some(money("111"))),
            Outcome::Fail
        );
        assert_eq!(
            invoice_total(None, Some(money("10")), Some(money("110"))),
            Outcome::Unknown
        );
        assert_eq!(critical_fields(&[true, false], true), Outcome::Unknown);
        assert_eq!(critical_fields(&[true], false), Outcome::Unknown);
    }
}
