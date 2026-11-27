use chrono::NaiveDate;
use finance_contracts::installments::{Installment, remaining};
use finance_financial::{Error, payment::Deduction};
use finance_shared::money::{Amount, Currency, Money};

fn money(value: &str) -> Money {
    Money {
        amount: Amount::parse(value).unwrap(),
        currency: Currency::parse("VND").unwrap(),
    }
}

#[test]
fn installment_applies_retention_advance_paid_and_hold_once() {
    let day = NaiveDate::from_ymd_opt(2027, 1, 15).unwrap();
    let item = Installment {
        effective_from: NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
        effective_until: NaiveDate::from_ymd_opt(2027, 2, 1).unwrap(),
        gross: Some(money("100")),
        confirmed_paid: money("20"),
        local_hold: money("10"),
        retention: Deduction::Separate(Some(money("10"))),
        advance_deduction: Deduction::Separate(Some(money("15"))),
        complete: true,
    };
    assert_eq!(remaining(item, day), Ok(money("45")));
}

#[test]
fn missing_deduction_or_stale_period_blocks() {
    let day = NaiveDate::from_ymd_opt(2027, 1, 15).unwrap();
    let mut item = Installment {
        effective_from: NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
        effective_until: NaiveDate::from_ymd_opt(2027, 2, 1).unwrap(),
        gross: Some(money("100")),
        confirmed_paid: money("0"),
        local_hold: money("0"),
        retention: Deduction::Separate(None),
        advance_deduction: Deduction::NotApplicable,
        complete: true,
    };
    assert_eq!(remaining(item, day), Err(Error::Unknown));
    item.retention = Deduction::NotApplicable;
    assert_eq!(
        remaining(item, NaiveDate::from_ymd_opt(2027, 2, 1).unwrap()),
        Err(Error::Unknown)
    );
}
