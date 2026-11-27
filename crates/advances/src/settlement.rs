//! Scoped multi-advance settlement proposal. No reservation or refund is posted here.
use crate::Advance;
use finance_financial::Error;
use finance_shared::{
    ids::{CompanyScope, ResourceId},
    money::{Amount, Money},
};
use std::collections::BTreeSet;

pub struct Allocation<'a> {
    pub scope: CompanyScope,
    pub id: ResourceId,
    pub advance: &'a Advance,
    pub requested: Money,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Proposal {
    pub applied: Money,
    pub extra_payment: Money,
    /// Amount still available against the selected advances, not a confirmed refund.
    pub unallocated: Money,
}

/// Inputs must come from a verified snapshot. The caller must reserve all
/// allocations atomically using the financial transaction port before handoff.
pub fn plan(
    scope: CompanyScope,
    expenses: Option<Money>,
    allocations: &[Allocation<'_>],
    extra_allowed: bool,
) -> Result<Proposal, Error> {
    let expenses = expenses.ok_or(Error::Unknown)?;
    if allocations.is_empty() || allocations.len() > 5000 {
        return Err(Error::Unknown);
    }
    if expenses.amount.decimal().is_sign_negative() {
        return Err(Error::Negative);
    }
    let zero = Money {
        amount: Amount::parse("0").map_err(|_| Error::Overflow)?,
        currency: expenses.currency,
    };
    let mut applied = zero;
    let mut unallocated = zero;
    let mut seen = BTreeSet::new();
    for row in allocations {
        if row.scope != scope || !seen.insert(row.id) {
            return Err(Error::Unknown);
        }
        let remaining = row.advance.remaining()?;
        if remaining.currency != expenses.currency || row.requested.currency != expenses.currency {
            return Err(Error::Currency);
        }
        if row.requested.amount.decimal() <= 0.into() {
            return Err(Error::Negative);
        }
        if row.requested.amount.decimal() > remaining.amount.decimal() {
            return Err(Error::ExceedsCap);
        }
        applied = applied
            .checked_add(row.requested)
            .map_err(|_| Error::Overflow)?;
        let rest = remaining
            .checked_sub(row.requested)
            .map_err(|_| Error::Overflow)?;
        unallocated = unallocated.checked_add(rest).map_err(|_| Error::Overflow)?;
    }
    if applied.amount.decimal() > expenses.amount.decimal() {
        return Err(Error::ExceedsCap);
    }
    let extra_payment = expenses
        .checked_sub(applied)
        .map_err(|_| Error::Overflow)?;
    if !extra_allowed && extra_payment != zero {
        return Err(Error::ExceedsCap);
    }
    Ok(Proposal {
        applied,
        extra_payment,
        unallocated,
    })
}
