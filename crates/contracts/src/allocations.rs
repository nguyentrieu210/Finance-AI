//! Verified acceptance allocation across dossiers.
use finance_financial::Error;
use finance_shared::{
    ids::{DossierId, ResourceId},
    money::{Amount, Money},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Acceptance {
    pub id: ResourceId,
    pub accepted: Money,
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Allocation {
    pub acceptance_id: ResourceId,
    pub dossier_id: DossierId,
    pub billed: Money,
    pub paid: Money,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Balance {
    pub acceptance_id: ResourceId,
    pub unbilled: Money,
    pub unpaid_billed: Money,
}

/// Reconcile verified receipt/acceptance capacity against dossier claims.
///
/// One acceptance may be split across dossiers, but the same acceptance+dossier
/// pair may only appear once. Aggregate billed amount cannot exceed accepted
/// value and confirmed paid cannot exceed billed. Missing/incomplete source
/// coverage blocks instead of assuming zero.
pub fn reconcile(
    acceptances: &[Acceptance],
    allocations: &[Allocation],
) -> Result<Vec<Balance>, Error> {
    if acceptances.is_empty() || acceptances.len() > 10_000 || allocations.len() > 50_000 {
        return Err(Error::Unknown);
    }
    let mut caps = BTreeMap::new();
    for row in acceptances {
        if !row.complete || row.accepted.amount.decimal().is_sign_negative() {
            return Err(Error::Unknown);
        }
        if caps.insert(row.id, row.accepted).is_some() {
            return Err(Error::Unknown);
        }
    }

    let mut totals: BTreeMap<ResourceId, (Money, Money)> = BTreeMap::new();
    let mut pairs = BTreeSet::new();
    for row in allocations {
        if !pairs.insert((row.acceptance_id, row.dossier_id)) {
            return Err(Error::Unknown);
        }
        let cap = *caps.get(&row.acceptance_id).ok_or(Error::Unknown)?;
        if row.billed.currency != cap.currency || row.paid.currency != cap.currency {
            return Err(Error::Currency);
        }
        if row.billed.amount.decimal().is_sign_negative()
            || row.paid.amount.decimal().is_sign_negative()
            || row.paid.amount.decimal() > row.billed.amount.decimal()
        {
            return Err(Error::Negative);
        }
        let zero = Money {
            amount: Amount::parse("0").map_err(|_| Error::Overflow)?,
            currency: cap.currency,
        };
        let entry = totals.entry(row.acceptance_id).or_insert((zero, zero));
        entry.0 = entry
            .0
            .checked_add(row.billed)
            .map_err(|_| Error::Overflow)?;
        entry.1 = entry.1.checked_add(row.paid).map_err(|_| Error::Overflow)?;
        if entry.0.amount.decimal() > cap.amount.decimal() {
            return Err(Error::ExceedsCap);
        }
    }

    caps.into_iter()
        .map(|(id, cap)| {
            let zero = Money {
                amount: Amount::parse("0").map_err(|_| Error::Overflow)?,
                currency: cap.currency,
            };
            let (billed, paid) = totals.get(&id).copied().unwrap_or((zero, zero));
            let unbilled = cap.checked_sub(billed).map_err(|_| Error::Overflow)?;
            let unpaid_billed = billed.checked_sub(paid).map_err(|_| Error::Overflow)?;
            Ok(Balance {
                acceptance_id: id,
                unbilled,
                unpaid_billed,
            })
        })
        .collect()
}
