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
    assert_eq!(resolve(&policies, scope, 1, day).err(), Some(Error::Conflict));
    policies[1].status = Status::Revoked;
    policies[0].checker = Some(policies[0].author);
    assert_eq!(resolve(&policies, scope, 1, day).err(), Some(Error::Invalid));
    policies[0].status = Status::Retired;
    assert_eq!(resolve(&policies, scope, 1, day).err(), Some(Error::Missing));
}
