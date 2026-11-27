#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
use chrono::NaiveDate;
use finance_financial::{Error, available};
use finance_shared::money::Money;

pub mod settlement;

pub struct Advance {
    pub issued: Option<Money>,
    pub settled: Money,
    pub confirmed_refund: Money,
    pub reserved: Money,
    pub due: NaiveDate,
    pub approved_extension: Option<NaiveDate>,
}

impl Advance {
    pub fn remaining(&self) -> Result<Money, Error> {
        let issued = self.issued.ok_or(Error::Unknown)?;
        if self.settled.amount.decimal().is_sign_negative()
            || self.confirmed_refund.amount.decimal().is_sign_negative()
        {
            return Err(Error::Negative);
        }
        if self.settled.currency != issued.currency
            || self.confirmed_refund.currency != issued.currency
            || self.reserved.currency != issued.currency
        {
            return Err(Error::Currency);
        }
        let consumed = self
            .settled
            .checked_add(self.confirmed_refund)
            .map_err(|_| Error::Overflow)?;
        available(issued, consumed, self.reserved, true)
    }

    pub fn overdue(&self, as_of: NaiveDate) -> Result<bool, Error> {
        let due = self
            .approved_extension
            .map_or(self.due, |d| d.max(self.due));
        // Holds restrict new allocations but are not confirmed settlement.
        let available = self.remaining()?;
        let outstanding = available
            .checked_add(self.reserved)
            .map_err(|_| Error::Overflow)?;
        Ok(as_of > due && outstanding.amount.decimal() > 0.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_shared::money::{Amount, Currency};

    #[test]
    fn unissued_and_confirmed_refund() {
        let m = |n| Money {
            amount: Amount::parse(n).unwrap(),
            currency: Currency::parse("VND").unwrap(),
        };
        let due = NaiveDate::from_ymd_opt(2026, 11, 27).unwrap();
        let mut a = Advance {
            issued: None,
            settled: m("20"),
            confirmed_refund: m("10"),
            reserved: m("5"),
            due,
            approved_extension: None,
        };
        assert_eq!(a.remaining(), Err(Error::Unknown));
        a.issued = Some(m("100"));
        assert_eq!(a.remaining(), Ok(m("65")));
        assert_eq!(a.overdue(due), Ok(false));
        assert_eq!(a.overdue(due.succ_opt().unwrap()), Ok(true));
        a.reserved = m("70");
        assert_eq!(a.remaining(), Ok(m("0")));
        assert_eq!(a.overdue(due.succ_opt().unwrap()), Ok(true));
    }
}
