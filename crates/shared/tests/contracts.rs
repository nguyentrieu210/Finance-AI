use finance_shared::{contracts::*, ids::*, money::*};
use uuid::Uuid;
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
#[test]
fn money_is_exact_and_wire_values_are_strings() {
    let a: Money = serde_json::from_str(r#"{"amount":"0.1","currency":"USD"}"#).unwrap();
    let b: Money = serde_json::from_str(r#"{"amount":"0.2","currency":"USD"}"#).unwrap();
    assert_eq!(
        serde_json::to_string(&a.checked_add(b).unwrap()).unwrap(),
        r#"{"amount":"0.3","currency":"USD"}"#
    );
    assert!(serde_json::from_str::<Money>(r#"{"amount":0.1,"currency":"USD"}"#).is_err());
    assert!(serde_json::from_str::<Money>(r#"{"amount":"1","currency":"usd"}"#).is_err());
}
#[test]
fn bounds_and_cross_currency_fail() {
    let max = Amount::parse("99999999999999999999.99999999").unwrap();
    assert!(
        max.checked_add(Amount::parse("0.00000001").unwrap())
            .is_err()
    );
    for value in [
        "100000000000000000000",
        "0.000000001",
        "1e3",
        "+1",
        "1.",
        " 1",
        "NaN",
    ] {
        assert!(Amount::parse(value).is_err(), "accepted {value}");
    }
    let usd = Money {
        amount: Amount::parse("1").unwrap(),
        currency: Currency::parse("USD").unwrap(),
    };
    let vnd = Money {
        currency: Currency::parse("VND").unwrap(),
        ..usd
    };
    assert_eq!(usd.checked_sub(vnd), Err(MoneyError::CurrencyMismatch));
    assert_eq!(
        Amount::parse("1")
            .unwrap()
            .checked_sub(Amount::parse("2").unwrap())
            .unwrap(),
        Amount::parse("-1").unwrap()
    );
}
#[test]
fn ids_and_versions_reject_invalid_wire_values() {
    assert!(TenantId::try_from(Uuid::nil()).is_err());
    let id = TenantId::try_from(id(1)).unwrap();
    assert_eq!(
        serde_json::from_str::<TenantId>(&serde_json::to_string(&id).unwrap()).unwrap(),
        id
    );
    assert!(serde_json::from_str::<Version>("-1").is_err());
    assert!(serde_json::from_str::<Version>("1.5").is_err());
}
#[test]
fn outcome_is_separate_from_severity() {
    assert_eq!(
        serde_json::to_string(&Outcome::Unknown).unwrap(),
        "\"UNKNOWN\""
    );
    assert_eq!(
        serde_json::to_string(&Outcome::NotApplicable).unwrap(),
        "\"NOT_APPLICABLE\""
    );
    assert_eq!(
        serde_json::to_string(&Severity::Block).unwrap(),
        "\"BLOCK\""
    );
}
#[test]
fn mismatched_company_or_tenant_denied_before_authorization() {
    let tenant = TenantId::try_from(id(1)).unwrap();
    let company = CompanyId::try_from(id(2)).unwrap();
    let scope = ResourceScope::Company(CompanyScope { tenant, company });
    let context = CommandContext {
        command: CommandId::try_from(id(3)).unwrap(),
        correlation: CorrelationId::try_from(id(4)).unwrap(),
        actor: Principal::User(ActorId::try_from(id(5)).unwrap()),
        scope,
        expected_version: Some(Version::try_from(0).unwrap()),
        occurred_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
    };
    let req = AuthorizationRequest {
        context,
        resource: ResourceId::try_from(id(6)).unwrap(),
        resource_scope: scope,
        action: Action::Read,
    };
    assert_eq!(check_scope(&req), Ok(())); // Scope agreement alone is NOT Allow.
    let different = ResourceScope::Company(CompanyScope {
        tenant,
        company: CompanyId::try_from(id(7)).unwrap(),
    });
    assert_eq!(
        check_scope(&AuthorizationRequest {
            resource_scope: different,
            ..req
        }),
        Err(PortError::ScopeMismatch)
    );
    assert_eq!(
        check_scope(&AuthorizationRequest {
            resource_scope: ResourceScope::Tenant(tenant),
            ..req
        }),
        Err(PortError::ScopeMismatch)
    );
}
