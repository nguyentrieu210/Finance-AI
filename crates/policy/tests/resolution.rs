use chrono::NaiveDate;
use finance_policy::{Error, Policy, Status, resolve};
use finance_shared::ids::{ActorId, CompanyId, CompanyScope, ResourceId, TenantId};
use uuid::Uuid;

fn fixture(id: u128, status: Status) -> Policy {
    Policy {
        id: ResourceId::try_from(Uuid::from_u128(id)).unwrap(),
        scope: CompanyScope {
            tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
            company: CompanyId::try_from(Uuid::from_u128(2)).unwrap(),
        },
        kind: 1,
        author: ActorId::try_from(Uuid::from_u128(3)).unwrap(),
        checker: Some(ActorId::try_from(Uuid::from_u128(4)).unwrap()),
        start: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        end_exclusive: NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
        clauses_mapped: true,
        status,
    }
}

#[test]
fn only_independently_checked_effective_version_can_resolve() {
    let p = fixture(5, Status::Published);
    let scope = p.scope;
    let day = p.start;
    let id = p.id;
    let mut policies = vec![p, fixture(6, Status::Draft)];
    assert_eq!(resolve(&policies, scope, 1, day).unwrap().id, id);
    policies[1].status = Status::Published;
    assert_eq!(
        resolve(&policies, scope, 1, day).err(),
        Some(Error::Conflict)
    );
    policies[1].status = Status::Revoked;
    policies[0].checker = Some(policies[0].author);
    assert_eq!(
        resolve(&policies, scope, 1, day).err(),
        Some(Error::Invalid)
    );
    policies[0].status = Status::Retired;
    assert_eq!(
        resolve(&policies, scope, 1, day).err(),
        Some(Error::Missing)
    );
}

#[test]
fn e16_at02_unmapped_policy_cannot_resolve() {
    let mut p = fixture(7, Status::Published);
    p.clauses_mapped = false;
    let scope = p.scope;
    let day = p.start;
    assert_eq!(resolve(&[p], scope, 1, day).err(), Some(Error::Invalid));
}

#[test]
fn e16_at03_effective_date_scope_and_kind_are_isolated() {
    let p = fixture(8, Status::Published);
    let scope = p.scope;
    let start = p.start;
    let end = p.end_exclusive;
    let mut foreign = fixture(9, Status::Published);
    foreign.scope.company = CompanyId::try_from(Uuid::from_u128(10)).unwrap();
    let mut other_kind = fixture(11, Status::Published);
    other_kind.kind = 2;
    let policies = [p, foreign, other_kind];
    assert_eq!(
        resolve(&policies, scope, 1, start).unwrap().id,
        policies[0].id
    );
    assert_eq!(
        resolve(&policies, scope, 1, start.pred_opt().unwrap()).err(),
        Some(Error::Missing)
    );
    assert_eq!(
        resolve(&policies, scope, 1, end.pred_opt().unwrap())
            .unwrap()
            .id,
        policies[0].id
    );
    assert_eq!(
        resolve(&policies, scope, 1, end).err(),
        Some(Error::Missing)
    );
}

#[test]
fn e16_at04_revoked_version_cannot_resolve_for_new_submission() {
    let p = fixture(12, Status::Revoked);
    let scope = p.scope;
    let day = p.start;
    assert_eq!(resolve(&[p], scope, 1, day).err(), Some(Error::Missing));
}
