#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
use finance_shared::{ids::CompanyScope, money::Money};

pub mod lines;
pub mod repository;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Match {
    Equal,
    Different,
    Unknown,
}

pub struct InvoiceIdentity<'a> {
    pub scope: CompanyScope,
    pub issuer: &'a str,
    pub series: &'a str,
    pub number: &'a str,
}

pub fn identity(a: &InvoiceIdentity<'_>, b: &InvoiceIdentity<'_>) -> Match {
    if a.scope != b.scope {
        return Match::Different;
    }
    if [a.issuer, a.series, a.number, b.issuer, b.series, b.number]
        .iter()
        .any(|s| s.trim().is_empty())
    {
        return Match::Unknown;
    }
    if (a.issuer, a.series, a.number) == (b.issuer, b.series, b.number) {
        Match::Equal
    } else {
        Match::Different
    }
}

/// Compare verified operands with an explicitly supplied nonnegative absolute
/// tolerance. No implicit currency or unit conversion.
pub fn amount(a: Option<Money>, b: Option<Money>, tolerance: Money) -> Match {
    let (Some(a), Some(b)) = (a, b) else {
        return Match::Unknown;
    };
    if a.currency != b.currency
        || a.currency != tolerance.currency
        || tolerance.amount.decimal().is_sign_negative()
    {
        return Match::Unknown;
    }
    match a.checked_sub(b) {
        Ok(delta) if delta.amount.decimal().abs() <= tolerance.amount.decimal() => Match::Equal,
        Ok(_) => Match::Different,
        Err(_) => Match::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_shared::money::{Amount, Currency};

    #[test]
    fn explicit_tolerance_and_currency() {
        let m = |n, c| Money {
            amount: Amount::parse(n).unwrap(),
            currency: Currency::parse(c).unwrap(),
        };
        assert_eq!(
            amount(Some(m("10", "VND")), Some(m("11", "VND")), m("1", "VND")),
            Match::Equal
        );
        assert_eq!(
            amount(Some(m("10", "VND")), Some(m("12", "VND")), m("1", "VND")),
            Match::Different
        );
        assert_eq!(
            amount(Some(m("10", "VND")), Some(m("10", "USD")), m("0", "VND")),
            Match::Unknown
        );
    }
}
