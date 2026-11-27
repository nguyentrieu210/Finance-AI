use finance_integration::{
    Observation,
    ports::{
        ContractError, DoCapabilities, DoReceipt, DoStatus, ErpEvent, ErpEventKind, ErpSnapshot,
        status_observation, validate_do_capabilities, validate_erp_snapshot,
    },
};
use finance_shared::{
    ids::{CompanyId, CompanyScope, ResourceId, TenantId},
    money::{Amount, Currency, Money},
    upload_contract::Sha256Digest,
};
use uuid::Uuid;

fn rid(n: u128) -> ResourceId {
    ResourceId::try_from(Uuid::from_u128(n)).unwrap()
}
fn scope() -> CompanyScope {
    CompanyScope {
        tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
        company: CompanyId::try_from(Uuid::from_u128(10)).unwrap(),
    }
}
fn digest(c: char) -> Sha256Digest {
    Sha256Digest::parse(&c.to_string().repeat(64)).unwrap()
}
fn money() -> Money {
    Money {
        amount: Amount::parse("125000").unwrap(),
        currency: Currency::parse("VND").unwrap(),
    }
}

#[test]
fn do_contract_requires_idempotency_and_authoritative_status() {
    assert_eq!(
        validate_do_capabilities(DoCapabilities {
            external_idempotency: false,
            signed_webhook: true,
            authenticated_polling: false,
        }),
        Err(ContractError::MissingIdempotency)
    );
    assert_eq!(
        validate_do_capabilities(DoCapabilities {
            external_idempotency: true,
            signed_webhook: false,
            authenticated_polling: false,
        }),
        Err(ContractError::MissingAuthoritativeStatusChannel)
    );
    assert!(validate_do_capabilities(DoCapabilities {
        external_idempotency: true,
        signed_webhook: true,
        authenticated_polling: false,
    })
    .is_ok());

    let base = DoReceipt {
        external_id: rid(20),
        status: DoStatus::Acknowledged,
        source_digest: digest('a'),
        authoritative: false,
    };
    assert_eq!(status_observation(base), Err(ContractError::UnverifiedStatus));
    assert_eq!(
        status_observation(DoReceipt {
            authoritative: true,
            ..base
        }),
        Ok(Some(Observation::ManualReceipt))
    );
    assert_eq!(
        status_observation(DoReceipt {
            status: DoStatus::Approved,
            authoritative: true,
            ..base
        }),
        Ok(Some(Observation::VerifiedApproval))
    );
}

#[test]
fn erp_snapshot_is_read_only_fresh_complete_and_replay_safe() {
    let payment = ErpEvent {
        external_id: rid(30),
        sequence: 1,
        kind: ErpEventKind::Payment,
        amount: money(),
        source_digest: digest('b'),
        reverses: None,
    };
    let reversal = ErpEvent {
        external_id: rid(31),
        sequence: 2,
        kind: ErpEventKind::Reversal,
        amount: money(),
        source_digest: digest('c'),
        reverses: Some(payment.external_id),
    };
    let snapshot = ErpSnapshot {
        scope: scope(),
        snapshot_id: rid(40),
        source_digest: digest('d'),
        as_of_unix_s: 100,
        fresh_until_unix_s: 200,
        complete: true,
        events: vec![payment, payment, reversal],
    };
    let normalized = validate_erp_snapshot(scope(), 150, &snapshot).unwrap();
    assert_eq!(normalized, vec![payment, reversal]);

    let mut stale = snapshot.clone();
    stale.fresh_until_unix_s = 120;
    assert_eq!(
        validate_erp_snapshot(scope(), 150, &stale),
        Err(ContractError::StaleSnapshot)
    );

    let mut incomplete = snapshot.clone();
    incomplete.complete = false;
    assert_eq!(
        validate_erp_snapshot(scope(), 150, &incomplete),
        Err(ContractError::IncompleteSnapshot)
    );

    let mut conflict = snapshot;
    conflict.events[1].source_digest = digest('e');
    assert_eq!(
        validate_erp_snapshot(scope(), 150, &conflict),
        Err(ContractError::DuplicateConflict)
    );
}
