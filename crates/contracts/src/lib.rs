#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
pub use finance_financial::references::{Reference, list_contracts as list_verified};
use chrono::NaiveDate;
use finance_financial::{Error, available};
use finance_shared::money::Money;

pub struct Amendment {
    pub effective: NaiveDate,
    pub approved: bool,
    pub delta: Money,
}

pub fn outstanding(
    base: Money,
    amendments: &[Amendment],
    date: NaiveDate,
    paid: Money,
    held: Money,
    complete: bool,
) -> Result<Money, Error> {
    if !complete {
        return Err(Error::Unknown);
    }
    let mut cap = base;
    for item in amendments
        .iter()
        .filter(|a| a.approved && a.effective <= date)
    {
        if item.delta.currency != base.currency {
            return Err(Error::Currency);
        }
        cap = cap.checked_add(item.delta).map_err(|_| Error::Overflow)?;
    }
    available(cap, paid, held, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_shared::money::{Amount, Currency};

    #[test]
    fn future_and_draft_amendments_excluded() {
        let m = |n| Money {
            amount: Amount::parse(n).unwrap(),
            currency: Currency::parse("VND").unwrap(),
        };
        let today = NaiveDate::from_ymd_opt(2026, 11, 27).unwrap();
        let tomorrow = today.succ_opt().unwrap();
        let changes = [
            Amendment {
                effective: tomorrow,
                approved: true,
                delta: m("100"),
            },
            Amendment {
                effective: today,
                approved: false,
                delta: m("100"),
            },
        ];
        assert_eq!(
            outstanding(m("100"), &changes, today, m("20"), m("10"), true),
            Ok(m("70"))
        );
        assert_eq!(
            outstanding(m("100"), &changes, today, m("20"), m("10"), false),
            Err(Error::Unknown)
        );
    }
}
