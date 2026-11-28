use chrono::NaiveDate;
use finance_advances::VerifiedIssue;
use finance_financial::Error;
use finance_shared::{
    ids::ResourceId,
    money::{Amount, Currency, Money},
};
use uuid::Uuid;

fn money(value: &str) -> Money {
    Money {
        amount: Amount::parse(value).unwrap(),
        currency: Currency::parse("VND").unwrap(),
    }
}

#[test]
fn verified_issue_requires_purpose_beneficiary_amount_and_source_verification() {
    let due = NaiveDate::from_ymd_opt(2027, 1, 31).unwrap();
    let beneficiary = ResourceId::try_from(Uuid::from_u128(7)).unwrap();
    let base = VerifiedIssue {
        purpose: "Đi công tác dự án",
        beneficiary,
        amount: Some(money("100")),
        due,
        source_verified: true,
    };
    let advance = base
        .into_advance(money("20"), money("10"), money("5"), None)
        .unwrap();
    assert_eq!(advance.remaining(), Ok(money("65")));

    assert_eq!(
        VerifiedIssue {
            purpose: " ",
            ..base
        }
        .into_advance(money("0"), money("0"), money("0"), None),
        Err(Error::Unknown)
    );
    assert_eq!(
        VerifiedIssue {
            source_verified: false,
            ..base
        }
        .into_advance(money("0"), money("0"), money("0"), None),
        Err(Error::Unknown)
    );
    assert_eq!(
        VerifiedIssue {
            amount: Some(money("0")),
            ..base
        }
        .into_advance(money("0"), money("0"), money("0"), None),
        Err(Error::Negative)
    );
}
