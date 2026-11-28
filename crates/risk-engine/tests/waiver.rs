use finance_risk_engine::{Finding, Waiver, blocks};
use finance_shared::{
    ids::{ActorId, ResourceId, RevisionId},
    upload_contract::Sha256Digest,
};
use uuid::Uuid;

#[test]
fn waiver_is_fenced_to_inputs_revision_and_independent_approver() {
    let id = ResourceId::try_from(Uuid::from_u128(1)).unwrap();
    let revision = RevisionId::try_from(Uuid::from_u128(2)).unwrap();
    let inputs = Sha256Digest::parse(&"a".repeat(64)).unwrap();
    let requester = ActorId::try_from(Uuid::from_u128(3)).unwrap();
    let approver = ActorId::try_from(Uuid::from_u128(4)).unwrap();
    let mut finding = Finding {
        id,
        revision,
        inputs,
        blocking: true,
        waivable: true,
        critical_unknown_or_error: false,
    };
    let mut waiver = Waiver {
        finding: id,
        revision,
        inputs,
        requester,
        approver,
        expires_at: 100,
        revoked: false,
        authorized: true,
    };
    assert!(!blocks(&finding, Some(&waiver), 99));
    assert!(blocks(&finding, Some(&waiver), 100));
    waiver.approver = requester;
    assert!(blocks(&finding, Some(&waiver), 99));
    waiver.approver = approver;
    finding.inputs = Sha256Digest::parse(&"b".repeat(64)).unwrap();
    assert!(blocks(&finding, Some(&waiver), 99));
    finding.inputs = inputs;
    finding.revision = RevisionId::try_from(Uuid::from_u128(5)).unwrap();
    assert!(blocks(&finding, Some(&waiver), 99));
    finding.revision = revision;
    waiver.revoked = true;
    assert!(blocks(&finding, Some(&waiver), 99));
    waiver.revoked = false;
    waiver.authorized = false;
    assert!(blocks(&finding, Some(&waiver), 99));
    waiver.authorized = true;
    assert!(blocks(&finding, None, 99));
    finding.waivable = false;
    assert!(blocks(&finding, Some(&waiver), 99));
    finding.waivable = true;
    finding.critical_unknown_or_error = true;
    assert!(blocks(&finding, Some(&waiver), 99));
}
