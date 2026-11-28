use chrono::NaiveDate;
use finance_advances::{
    Advance,
    settlement::{Allocation, Proposal, plan},
};
use finance_financial::Error;
use finance_shared::{
    ids::{CompanyId, CompanyScope, ResourceId, TenantId},
    money::{Amount, Currency, Money},
};
use uuid::Uuid;

fn scope(n: u128) -> CompanyScope {
    CompanyScope {
        tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
        company: CompanyId::try_from(Uuid::from_u128(n)).unwrap(),
    }
}

fn id(n: u128) -> ResourceId {
    ResourceId::try_from(Uuid::from_u128(n)).unwrap()
}

fn money(n: &str) -> Money {
    Money {
        amount: Amount::parse(n).unwrap(),
        currency: Currency::parse("VND").unwrap(),
    }
}

fn advance() -> Advance {
    Advance {
        issued: Some(money("100")),
        settled: money("20"),
        confirmed_refund: money("10"),
        reserved: money("15"),
        due: NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
        approved_extension: None,
    }
}

#[test]
fn partial_multi_advance_and_separate_extra_payment() {
    let a = advance();
    let b = advance();
    let mut rows = [
        Allocation {
            scope: scope(10),
            id: id(20),
            advance: &a,
            requested: money("40"),
        },
        Allocation {
            scope: scope(10),
            id: id(21),
            advance: &b,
            requested: money("30"),
        },
    ];
    assert_eq!(
        plan(scope(10), Some(money("80")), &rows, true),
        Ok(Proposal {
            applied: money("70"),
            extra_payment: money("10"),
            unallocated: money("40"),
        })
    );
    assert_eq!(
        plan(scope(10), Some(money("80")), &rows, false),
        Err(Error::ExceedsCap)
    );
    rows[1].id = id(20);
    assert_eq!(
        plan(scope(10), Some(money("80")), &rows, true),
        Err(Error::Unknown)
    );
    rows[1].id = id(21);
    rows[1].scope = scope(11);
    assert_eq!(
        plan(scope(10), Some(money("80")), &rows, true),
        Err(Error::Unknown)
    );
    rows[1].scope = scope(10);
    rows[1].requested = money("56");
    assert_eq!(
        plan(scope(10), Some(money("100")), &rows, true),
        Err(Error::ExceedsCap)
    );
}

#[test]
fn e13_at01_draft_advance_is_not_available_for_settlement() {
    let mut a = advance();
    a.issued = None;
    let rows = [Allocation {
        scope: scope(10),
        id: id(20),
        advance: &a,
        requested: money("1"),
    }];
    assert_eq!(
        plan(scope(10), Some(money("1")), &rows, true),
        Err(Error::Unknown)
    );
}

#[test]
fn e13_r02_rejects_currency_mismatch_without_approved_fx() {
    let a = advance();
    let usd = Money {
        amount: Amount::parse("1").unwrap(),
        currency: Currency::parse("USD").unwrap(),
    };
    let rows = [Allocation {
        scope: scope(10),
        id: id(20),
        advance: &a,
        requested: usd,
    }];
    assert_eq!(
        plan(scope(10), Some(money("1")), &rows, true),
        Err(Error::Currency)
    );
}

#[test]
fn e13_at04_approved_extension_and_business_date_boundary() {
    let mut a = advance();
    let original_due = a.due;
    let extended = original_due.succ_opt().unwrap();
    a.approved_extension = Some(extended);
    assert_eq!(a.overdue(original_due), Ok(false));
    assert_eq!(a.overdue(extended), Ok(false));
    assert_eq!(a.overdue(extended.succ_opt().unwrap()), Ok(true));

    // Full confirmed settlement clears overdue; a hold alone does not.
    a.settled = money("90");
    a.reserved = money("0");
    assert_eq!(a.overdue(extended.succ_opt().unwrap()), Ok(false));
}
