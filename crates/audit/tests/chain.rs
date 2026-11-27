use finance_audit::*;
use uuid::Uuid;
fn event(n: i64, prev: [u8; 32]) -> Event {
    let mut e = Event {
        tenant: Uuid::from_u128(1),
        company: Uuid::from_u128(10),
        id: Uuid::from_u128(n as u128),
        actor: Uuid::from_u128(5),
        command: Uuid::from_u128(n as u128),
        correlation: Uuid::from_u128(7),
        resource: Uuid::from_u128(8),
        sequence: n,
        unix_micros: 42,
        action: "ORG_UNIT_CREATED".into(),
        previous: prev,
        digest: [0; 32],
    };
    e.digest = e.calculate_digest();
    e
}
#[test]
fn detects_modification_gap_and_truncation() {
    let a = event(1, [0; 32]);
    let b = event(2, a.digest);
    let head = Head {
        sequence: 2,
        digest: b.digest,
    };
    assert_eq!(verify_chain(a.tenant, [a.clone(), b.clone()], head), Ok(()));
    let mut changed = a.clone();
    changed.action = "AUDIT_READ".into();
    assert_eq!(
        verify_chain(a.tenant, [changed, b.clone()], head),
        Err(IntegrityError::DigestMismatch)
    );
    assert_eq!(
        verify_chain(a.tenant, [b.clone()], head),
        Err(IntegrityError::Gap)
    );
    assert_eq!(
        verify_chain(a.tenant, [a.clone()], head),
        Err(IntegrityError::HeadMismatch)
    );
    assert_eq!(
        verify_chain(Uuid::from_u128(2), [a, b], head),
        Err(IntegrityError::WrongTenant)
    );
}
