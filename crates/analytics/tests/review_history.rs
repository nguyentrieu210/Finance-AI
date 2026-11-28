use finance_analytics::review_history::{CohortCase, Counts, Decision, Error, Event, calculate};
use finance_shared::ids::{CompanyId, CompanyScope, DossierId, ResourceId, TenantId};
use uuid::Uuid;

fn scope(company: u128) -> CompanyScope {
    CompanyScope {
        tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
        company: CompanyId::try_from(Uuid::from_u128(company)).unwrap(),
    }
}

fn case(n: u128, event_count: u32) -> CohortCase {
    CohortCase {
        scope: scope(2),
        dossier: DossierId::try_from(Uuid::from_u128(n)).unwrap(),
        event_count,
        history_complete: true,
    }
}

fn event(case: CohortCase, n: u128, sequence: u32, decision: Decision) -> Event {
    Event {
        scope: case.scope,
        dossier: case.dossier,
        id: ResourceId::try_from(Uuid::from_u128(n)).unwrap(),
        sequence,
        occurred_unix_s: u64::from(sequence) * 10,
        decision,
    }
}

#[test]
fn first_pass_excludes_reopened_and_replayed_cases() {
    let clean = case(10, 1);
    let returned = case(11, 3);
    let pending = case(12, 0);
    let accepted = event(clean, 100, 1, Decision::Accepted);
    let first = event(returned, 101, 1, Decision::Returned);
    let reopen = event(returned, 102, 2, Decision::Reopened);
    let last = event(returned, 103, 3, Decision::Accepted);
    let foreign = Event {
        scope: scope(3),
        ..accepted
    };
    assert_eq!(
        calculate(
            scope(2),
            &[clean, returned, pending, clean],
            &[last, first, accepted, reopen, first, foreign],
            100
        ),
        Ok(Counts {
            cases: 3,
            accepted: 2,
            first_pass_accepted: 1,
            returned_cases: 1,
            not_accepted: 1
        })
    );
}

#[test]
fn rejects_gaps_conflicts_and_impossible_transitions() {
    let case = case(10, 2);
    let first = event(case, 100, 1, Decision::Returned);
    let second = event(case, 101, 2, Decision::Accepted);
    assert_eq!(
        calculate(scope(2), &[case], &[second], 100),
        Err(Error::IncompleteHistory)
    );
    assert_eq!(
        calculate(scope(2), &[case], &[first, second], 100),
        Err(Error::InvalidTransition)
    );
    let replay = Event {
        decision: Decision::Reopened,
        ..first
    };
    assert_eq!(
        calculate(scope(2), &[case], &[first, replay], 100),
        Err(Error::ConflictingReplay)
    );
    assert_eq!(
        calculate(scope(2), &[case], &[first, second], 5),
        Err(Error::InvalidTime)
    );
}
