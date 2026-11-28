use finance_integration::{
    External, Observation, may_release,
    reconcile::{Apply, Error, Event, Reconciler},
};
use finance_shared::{
    ids::{CompanyId, CompanyScope, ResourceId, RevisionId, TenantId},
    upload_contract::Sha256Digest,
};
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
        scope,
        handoff,
        revision,
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
    assert_eq!(
        state.apply(Event {
            sequence: 3,
            ..next
        }),
        Err(Error::ReplayConflict)
    );
}

#[test]
fn e23_at02_out_of_order_and_cross_binding_callbacks_fail_closed() {
    let scope = CompanyScope {
        tenant: TenantId::try_from(Uuid::from_u128(11)).unwrap(),
        company: CompanyId::try_from(Uuid::from_u128(12)).unwrap(),
    };
    let handoff = ResourceId::try_from(Uuid::from_u128(13)).unwrap();
    let revision = RevisionId::try_from(Uuid::from_u128(14)).unwrap();
    let mut state = Reconciler::new(scope, handoff, revision);
    let timeout = Event {
        scope,
        handoff,
        revision,
        external_id: ResourceId::try_from(Uuid::from_u128(15)).unwrap(),
        sequence: 2,
        source_digest: Sha256Digest::parse(&"b".repeat(64)).unwrap(),
        observation: Observation::Timeout,
        independently_verified: false,
    };

    assert_eq!(state.apply(timeout), Ok(Apply::Applied));
    assert_eq!(state.state(), External::Unknown);
    assert!(!may_release(state.state(), true));

    // An older callback with a different external ID is stale, not a new event.
    let out_of_order = Event {
        external_id: ResourceId::try_from(Uuid::from_u128(16)).unwrap(),
        sequence: 1,
        ..timeout
    };
    assert_eq!(state.apply(out_of_order), Err(Error::StaleSequence));
    assert_eq!(state.state(), External::Unknown);

    // Events from another company or dossier revision cannot advance this handoff.
    let wrong_scope = Event {
        scope: CompanyScope {
            tenant: scope.tenant,
            company: CompanyId::try_from(Uuid::from_u128(99)).unwrap(),
        },
        external_id: ResourceId::try_from(Uuid::from_u128(17)).unwrap(),
        sequence: 3,
        observation: Observation::ManualReceipt,
        independently_verified: true,
        ..timeout
    };
    assert_eq!(state.apply(wrong_scope), Err(Error::WrongBinding));
    assert_eq!(state.state(), External::Unknown);

    let wrong_revision = Event {
        revision: RevisionId::try_from(Uuid::from_u128(98)).unwrap(),
        external_id: ResourceId::try_from(Uuid::from_u128(18)).unwrap(),
        sequence: 3,
        observation: Observation::ManualReceipt,
        independently_verified: true,
        ..timeout
    };
    assert_eq!(state.apply(wrong_revision), Err(Error::WrongBinding));
    assert_eq!(state.state(), External::Unknown);

    // Rejected callbacks did not consume sequence 3; a correctly bound receipt can apply.
    let receipt = Event {
        external_id: ResourceId::try_from(Uuid::from_u128(17)).unwrap(),
        sequence: 3,
        observation: Observation::ManualReceipt,
        independently_verified: true,
        ..timeout
    };
    assert_eq!(state.apply(receipt), Ok(Apply::Applied));
    assert_eq!(state.state(), External::Acknowledged);
    assert!(!may_release(state.state(), true));
}
