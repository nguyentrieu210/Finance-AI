#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
use chrono::NaiveDate;
pub use finance_financial::references::{Reference, list_budgets as list_verified};
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

/// A budget snapshot must be pinned to the company, account, cost center and
/// approved revision before its available amount can be used.
pub struct BudgetReference<'a> {
    pub budget: Budget,
    pub scope: finance_shared::ids::CompanyScope,
    pub cost_center: &'a str,
    pub account: &'a str,
    pub version: u64,
    pub as_of: NaiveDate,
    pub fresh_until: NaiveDate,
    pub approved: bool,
}

pub struct BudgetQuery<'a> {
    pub scope: finance_shared::ids::CompanyScope,
    pub cost_center: &'a str,
    pub account: &'a str,
    pub version: u64,
    pub business_day: NaiveDate,
    pub observed_day: NaiveDate,
}

impl BudgetReference<'_> {
    pub fn verified_available(&self, query: BudgetQuery<'_>) -> Result<Money, Error> {
        if !self.approved
            || self.scope != query.scope
            || self.version == 0
            || self.version != query.version
            || self.cost_center.trim().is_empty()
            || self.account.trim().is_empty()
            || self.cost_center != query.cost_center
            || self.account != query.account
            || self.as_of > query.observed_day
            || self.fresh_until <= query.observed_day
            || self.fresh_until <= self.as_of
            || query.business_day > query.observed_day
        {
            return Err(Error::Unknown);
        }
        self.budget.available_on(query.business_day)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_shared::money::{Amount, Currency};

    #[test]
    fn e14_at01_budget_snapshot_requires_matching_scope_version_and_freshness() {
        use finance_shared::ids::{CompanyId, CompanyScope, TenantId};
        use uuid::Uuid;
        let scope = CompanyScope {
            tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
            company: CompanyId::try_from(Uuid::from_u128(2)).unwrap(),
        };
        let m = |n| Money {
            amount: Amount::parse(n).unwrap(),
            currency: Currency::parse("VND").unwrap(),
        };
        let start = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        let reference = BudgetReference {
            budget: Budget {
                start,
                end_exclusive: end,
                cap: m("100"),
                confirmed_actual: m("50"),
                unmatched_hold: m("20"),
                reconciled: true,
            },
            scope,
            cost_center: "CC01",
            account: "621",
            version: 2,
            as_of: start,
            fresh_until: end,
            approved: true,
        };
        let query = |version, account, observed_day| BudgetQuery {
            scope,
            cost_center: "CC01",
            account,
            version,
            business_day: day,
            observed_day,
        };
        assert_eq!(
            reference.verified_available(query(2, "621", day)),
            Ok(m("30"))
        );
        assert_eq!(
            reference.verified_available(query(1, "621", day)),
            Err(Error::Unknown)
        );
        assert_eq!(
            reference.verified_available(query(2, "622", day)),
            Err(Error::Unknown)
        );
        assert_eq!(
            reference.verified_available(query(2, "621", end)),
            Err(Error::Unknown)
        );
        let other_scope = CompanyScope {
            company: CompanyId::try_from(Uuid::from_u128(3)).unwrap(),
            ..scope
        };
        assert_eq!(
            reference.verified_available(BudgetQuery {
                scope: other_scope,
                ..query(2, "621", day)
            }),
            Err(Error::Unknown)
        );
    }

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
