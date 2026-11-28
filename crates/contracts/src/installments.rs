//! Installment-level payable calculation with explicit deductions.
use chrono::NaiveDate;
use finance_financial::{
    Error, available,
    payment::{Deduction, net},
};
use finance_shared::money::Money;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Installment {
    pub effective_from: NaiveDate,
    pub effective_until: NaiveDate,
    pub gross: Option<Money>,
    pub confirmed_paid: Money,
    pub local_hold: Money,
    pub retention: Deduction,
    pub advance_deduction: Deduction,
    pub complete: bool,
}

/// Remaining amount for one verified installment on the business date.
///
/// Retention and advance deductions are never inferred. Missing applicable
/// deductions, stale/incomplete coverage, invalid effective dates, or currency
/// mismatches block the calculation.
pub fn remaining(item: Installment, day: NaiveDate) -> Result<Money, Error> {
    if !item.complete
        || item.effective_from >= item.effective_until
        || day < item.effective_from
        || day >= item.effective_until
    {
        return Err(Error::Unknown);
    }
    let eligible = net(
        item.gross,
        Deduction::NotApplicable,
        item.retention,
        item.advance_deduction,
        Deduction::NotApplicable,
    )?;
    available(eligible, item.confirmed_paid, item.local_hold, true)
}
