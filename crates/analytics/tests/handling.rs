use finance_analytics::handling::{CohortCase, Error, Interval, calculate};
use finance_shared::ids::{CompanyId, CompanyScope, DossierId, TenantId};
use uuid::Uuid;

fn scope(company: u128) -> CompanyScope {
    CompanyScope {
        tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
        company: CompanyId::try_from(Uuid::from_u128(company)).unwrap(),
    }
}

fn dossier(n: u128) -> DossierId {
    DossierId::try_from(Uuid::from_u128(n)).unwrap()
}

#[test]
fn e24_r01_active_intervals_are_deduplicated_and_scope_bound() {
    let selected = scope(2);
    let case = CohortCase {
        scope: selected,
        dossier: dossier(10),
        history_complete: true,
    };
    let first = Interval {
        scope: selected,
        dossier: case.dossier,
        start_unix_s: 10,
        end_unix_s: 30,
    };
    let overlap = Interval {
        start_unix_s: 20,
        end_unix_s: 40,
        ..first
    };
    let later = Interval {
        start_unix_s: 60,
        end_unix_s: 70,
        ..first
    };
    let foreign = Interval {
        scope: scope(3),
        start_unix_s: 1,
        end_unix_s: 99,
        ..first
    };
    let result = calculate(
        selected,
        &[case, case],
        &[later, first, overlap, first, foreign],
        100,
    )
    .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].active_seconds, 40);
}

#[test]
fn e24_r01_incomplete_or_invalid_history_is_not_a_zero_kpi() {
    let selected = scope(2);
    let case = CohortCase {
        scope: selected,
        dossier: dossier(10),
        history_complete: false,
    };
    assert_eq!(
        calculate(selected, &[case], &[], 100),
        Err(Error::IncompleteHistory)
    );
    let case = CohortCase {
        history_complete: true,
        ..case
    };
    let invalid = Interval {
        scope: selected,
        dossier: case.dossier,
        start_unix_s: 99,
        end_unix_s: 101,
    };
    assert_eq!(
        calculate(selected, &[case], &[invalid], 100),
        Err(Error::InvalidInterval)
    );
    let outside = Interval {
        dossier: dossier(11),
        start_unix_s: 10,
        end_unix_s: 20,
        ..invalid
    };
    assert_eq!(
        calculate(selected, &[case], &[outside], 100),
        Err(Error::OutsideCohort)
    );
}
