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
