use finance_contracts::allocations::{Acceptance, Allocation, reconcile};
use finance_financial::Error;
use finance_shared::{
    ids::{DossierId, ResourceId},
    money::{Amount, Currency, Money},
};
use uuid::Uuid;

fn money(value: &str) -> Money {
    Money {
        amount: Amount::parse(value).unwrap(),
        currency: Currency::parse("VND").unwrap(),
    }
}
fn resource(value: u128) -> ResourceId {
    ResourceId::try_from(Uuid::from_u128(value)).unwrap()
}
fn dossier(value: u128) -> DossierId {
    DossierId::try_from(Uuid::from_u128(value)).unwrap()
}

#[test]
fn split_acceptance_is_bounded_and_duplicate_claim_is_rejected() {
    let accepted = [Acceptance {
        id: resource(1),
        accepted: money("100"),
        complete: true,
    }];
    let rows = [
        Allocation {
            acceptance_id: resource(1),
            dossier_id: dossier(10),
            billed: money("40"),
            paid: money("20"),
        },
        Allocation {
            acceptance_id: resource(1),
            dossier_id: dossier(11),
            billed: money("30"),
            paid: money("10"),
        },
    ];
    let result = reconcile(&accepted, &rows).unwrap();
    assert_eq!(result[0].unbilled, money("30"));
    assert_eq!(result[0].unpaid_billed, money("40"));

    let duplicate = [rows[0], rows[0]];
    assert_eq!(reconcile(&accepted, &duplicate), Err(Error::Unknown));

    let over = [
        rows[0],
        Allocation {
            billed: money("61"),
            ..rows[1]
        },
    ];
    assert_eq!(reconcile(&accepted, &over), Err(Error::ExceedsCap));
}

#[test]
fn incomplete_or_unknown_acceptance_fails_closed() {
    let incomplete = [Acceptance {
        id: resource(1),
        accepted: money("100"),
        complete: false,
    }];
    assert_eq!(reconcile(&incomplete, &[]), Err(Error::Unknown));
    let rows = [Allocation {
        acceptance_id: resource(2),
        dossier_id: dossier(10),
        billed: money("1"),
        paid: money("0"),
    }];
    let complete = [Acceptance {
        id: resource(1),
        accepted: money("100"),
        complete: true,
    }];
    assert_eq!(reconcile(&complete, &rows), Err(Error::Unknown));
}
