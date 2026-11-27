#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
pub use finance_financial::references::{Reference, list_budgets as list_verified};
use chrono::NaiveDate;
use finance_financial::{Error, available};
use finance_shared::money::Money;

pub struct Budget {
    pub start: NaiveDate,
    pub end_exclusive: NaiveDate,
    pub cap: Money,
    pub confirmed_actual: Money,
    pub unmatched_hold: Money,
    pub reconciled: bool,
}

impl Budget {
    pub fn available_on(&self, day: NaiveDate) -> Result<Money, Error> {
        if day < self.start || day >= self.end_exclusive || self.start >= self.end_exclusive {
            return Err(Error::Unknown);
        }
        available(
            self.cap,
            self.confirmed_actual,
            self.unmatched_hold,
            self.reconciled,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_shared::money::{Amount, Currency};

    #[test]
    fn no_stale_or_unreconciled_budget() {
        let m = |n| Money {
            amount: Amount::parse(n).unwrap(),
            currency: Currency::parse("VND").unwrap(),
        };
        let start = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        let mut b = Budget {
            start,
            end_exclusive: end,
            cap: m("100"),
            confirmed_actual: m("60"),
            unmatched_hold: m("20"),
            reconciled: false,
        };
        assert_eq!(b.available_on(start), Err(Error::Unknown));
        b.reconciled = true;
        assert_eq!(b.available_on(start), Ok(m("20")));
        assert_eq!(b.available_on(end), Err(Error::Unknown));
    }
}
