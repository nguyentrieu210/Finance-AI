use finance_integration::{External, Observation, may_release, reconcile::{Apply, Error, Event, Reconciler}};
use finance_shared::{ids::{CompanyId, CompanyScope, ResourceId, RevisionId, TenantId}, upload_contract::Sha256Digest};
use uuid::Uuid;

#[test]
fn timeout_replay_and_independent_reconciliation() {
    let scope = CompanyScope {
        tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
        company: CompanyId::try_from(Uuid::from_u128(2)).unwrap(),
    };
    let handoff = ResourceId::try_from(Uuid::from_u128(3)).unwrap();
    let revision = RevisionId::try_from(Uuid::from_u128(4)).unwrap();
    let mut state = Reconciler::new(scope, handoff, revision);
    let event = Event {
        scope, handoff, revision,
        external_id: ResourceId::try_from(Uuid::from_u128(5)).unwrap(),
        sequence: 1,
        source_digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        observation: Observation::Timeout,
        independently_verified: false,
    };
    assert_eq!(state.apply(event), Ok(Apply::Applied));
    assert_eq!(state.state(), External::Unknown);
    assert!(!may_release(state.state(), true));
    assert_eq!(state.apply(event), Ok(Apply::Replayed));
    let mut next = Event {
        external_id: ResourceId::try_from(Uuid::from_u128(6)).unwrap(),
        sequence: 2,
        observation: Observation::VerifiedApproval,
        ..event
    };
    assert_eq!(state.apply(next), Err(Error::InvalidTransition));
    next.independently_verified = true;
    assert_eq!(state.apply(next), Ok(Apply::Applied));
    assert_eq!(state.state(), External::Approved);
    assert!(!may_release(state.state(), true));
    assert_eq!(state.apply(Event { sequence: 3, ..next }), Err(Error::ReplayConflict));
}
