use finance_shared::{contracts::Version, http_contract::*, ids::ResourceId};
use uuid::Uuid;
fn resource(n: u128) -> ResourceId {
    ResourceId::try_from(Uuid::from_u128(n)).unwrap()
}
#[test]
fn strong_version_tokens_are_canonical_and_target_bound() {
    let token = VersionToken {
        target: VersionTarget::Dossier,
        resource_id: resource(1),
        version: Version::try_from(17).unwrap(),
    };
    let encoded = token.to_string();
    assert_eq!(VersionToken::parse(&encoded), Ok(token));
    assert_eq!(
        token.check(
            VersionTarget::Dossier,
            resource(1),
            Version::try_from(17).unwrap()
        ),
        Ok(())
    );
    assert_eq!(
        token.check(
            VersionTarget::Dossier,
            resource(2),
            Version::try_from(17).unwrap()
        ),
        Err(HeaderError::WrongTarget)
    );
    assert_eq!(
        token.check(
            VersionTarget::Policy,
            resource(1),
            Version::try_from(17).unwrap()
        ),
        Err(HeaderError::WrongTarget)
    );
    assert_eq!(
        token.check(
            VersionTarget::Dossier,
            resource(1),
            Version::try_from(18).unwrap()
        ),
        Err(HeaderError::StaleVersion)
    );
}
#[test]
fn reject_weak_wildcard_lists_and_unsafe_versions() {
    let id = resource(1);
    for bad in [
        "*".to_owned(),
        format!("W/\"v1:dossier:{id}:0\""),
        format!("\"v1:dossier:{id}:0\", \"v1:dossier:{id}:1\""),
        format!("\"v1:dossier:{id}:00\""),
        format!("\"v1:dossier:{id}:+1\""),
        format!("\"v1:dossier:{id}:9007199254740992\""),
        format!("\"v1:unknown:{id}:1\""),
        format!("\"v1:dossier:{id}:1\" "),
    ] {
        assert!(
            VersionToken::parse(&bad).is_err(),
            "accepted invalid token {bad}"
        );
    }
    assert!(VersionToken::parse(&format!("\"v1:dossier:{id}:9007199254740991\"")).is_ok());
    assert!(serde_json::from_str::<Version>("9007199254740992").is_err());
}
#[test]
fn idempotency_keys_are_bounded_ascii_and_redacted() {
    let key = IdempotencyKey::parse("request-1._~").unwrap();
    assert_eq!(key.as_str(), "request-1._~");
    assert!(!format!("{key:?}").contains("request-1"));
    assert!(IdempotencyKey::parse(&"a".repeat(128)).is_ok());
    assert!(IdempotencyKey::parse(&"a".repeat(129)).is_err());
    for bad in ["", "x y", "x\r\n", "đ", "*", "a,b"] {
        assert!(IdempotencyKey::parse(bad).is_err());
    }
}
