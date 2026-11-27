use finance_knowledge::{Clause, retrieve};
use finance_shared::ids::{CompanyId, CompanyScope, ResourceId, TenantId};
use uuid::Uuid;

#[test]
fn retrieval_excludes_draft_and_cross_company() {
    let scope = CompanyScope {
        tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
        company: CompanyId::try_from(Uuid::from_u128(2)).unwrap(),
    };
    let other = CompanyScope {
        company: CompanyId::try_from(Uuid::from_u128(3)).unwrap(),
        ..scope
    };
    let version = ResourceId::try_from(Uuid::from_u128(4)).unwrap();
    let clause = |id, scope, approved_current| Clause {
        scope,
        version,
        id: ResourceId::try_from(Uuid::from_u128(id)).unwrap(),
        approved_current,
        text: "Quy trình thanh toán thử nghiệm",
    };
    let items = [
        clause(5, scope, true),
        clause(6, scope, false),
        clause(7, other, true),
    ];
    let result = retrieve(&items, scope, version, "thanh toán");
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, items[0].id);
    assert!(retrieve(&items, scope, version, "").is_empty());
}
