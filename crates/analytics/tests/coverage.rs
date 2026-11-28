use finance_analytics::coverage::{Coverage, Observation, Outcome, calculate};
use finance_shared::ids::{CompanyId, CompanyScope, DossierId, ResourceId, TenantId};
use uuid::Uuid;

fn scope(n: u128) -> CompanyScope {
    CompanyScope {
        tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
        company: CompanyId::try_from(Uuid::from_u128(n)).unwrap(),
    }
}

fn row(n: u128, outcome: Outcome) -> Observation {
    Observation {
        scope: scope(10),
        dossier: DossierId::try_from(Uuid::from_u128(20)).unwrap(),
        criterion: ResourceId::try_from(Uuid::from_u128(n)).unwrap(),
        outcome,
        automatic: true,
        evidenced: true,
    }
}

#[test]
fn coverage_deduplicates_and_never_counts_unknown_as_automation() {
    let pass = row(1, Outcome::Pass);
    let mut foreign = row(2, Outcome::Pass);
    foreign.scope = scope(11);
    let mut unproven = row(3, Outcome::NotApplicable);
    unproven.evidenced = false;
    let rows = [
        pass,
        pass,
        foreign,
        unproven,
        row(4, Outcome::Unknown),
        row(5, Outcome::NotApplicable),
        row(6, Outcome::Fail),
        row(7, Outcome::Error),
    ];
    assert_eq!(
        calculate(scope(10), &rows),
        Some(Coverage {
            applicable: 5,
            automatic_evidenced: 2,
            unknown_or_error: 3,
            excluded_not_applicable: 1,
        })
    );
    let conflict = row(1, Outcome::Fail);
    assert_eq!(calculate(scope(10), &[pass, conflict]), None);
}
