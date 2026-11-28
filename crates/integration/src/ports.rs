//! E23 live-adapter boundary. These contracts do not contain credentials or network
//! implementations. X01 must supply approved provider documentation before a live
//! DO/ERP adapter can implement them.
use crate::Observation;
use finance_shared::{
    ids::{CompanyScope, ResourceId, RevisionId},
    money::Money,
    upload_contract::Sha256Digest,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DoCapabilities {
    pub external_idempotency: bool,
    pub signed_webhook: bool,
    pub authenticated_polling: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DoStatus {
    Pending,
    Unknown,
    Acknowledged,
    Approved,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DoDispatch {
    pub scope: CompanyScope,
    pub handoff: ResourceId,
    pub revision: RevisionId,
    pub command_id: ResourceId,
    pub payload_digest: Sha256Digest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DoReceipt {
    pub external_id: ResourceId,
    pub status: DoStatus,
    pub source_digest: Sha256Digest,
    pub authoritative: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErpEventKind {
    Payment,
    Settlement,
    Reversal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ErpEvent {
    pub external_id: ResourceId,
    pub sequence: u64,
    pub kind: ErpEventKind,
    pub amount: Money,
    pub source_digest: Sha256Digest,
    pub reverses: Option<ResourceId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErpSnapshot {
    pub scope: CompanyScope,
    pub snapshot_id: ResourceId,
    pub source_digest: Sha256Digest,
    pub as_of_unix_s: u64,
    pub fresh_until_unix_s: u64,
    pub complete: bool,
    pub events: Vec<ErpEvent>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractError {
    MissingIdempotency,
    MissingAuthoritativeStatusChannel,
    UnverifiedStatus,
    WrongScope,
    IncompleteSnapshot,
    StaleSnapshot,
    DuplicateConflict,
    InvalidReversal,
    InvalidAmount,
    SequenceRegression,
}

pub trait DoPort {
    type Error;
    fn capabilities(&self) -> Result<DoCapabilities, Self::Error>;
    fn dispatch(&self, command: DoDispatch) -> Result<DoReceipt, Self::Error>;
    fn lookup(&self, external_id: ResourceId) -> Result<DoReceipt, Self::Error>;
}

pub trait ErpReadPort {
    type Error;
    fn snapshot(&self, scope: CompanyScope) -> Result<ErpSnapshot, Self::Error>;
}

pub fn validate_do_capabilities(value: DoCapabilities) -> Result<(), ContractError> {
    if !value.external_idempotency {
        return Err(ContractError::MissingIdempotency);
    }
    if !value.signed_webhook && !value.authenticated_polling {
        return Err(ContractError::MissingAuthoritativeStatusChannel);
    }
    Ok(())
}

pub fn status_observation(receipt: DoReceipt) -> Result<Option<Observation>, ContractError> {
    match receipt.status {
        DoStatus::Pending => Ok(None),
        DoStatus::Unknown => Ok(Some(Observation::Timeout)),
        DoStatus::Acknowledged if receipt.authoritative => Ok(Some(Observation::ManualReceipt)),
        DoStatus::Approved if receipt.authoritative => Ok(Some(Observation::VerifiedApproval)),
        DoStatus::Rejected if receipt.authoritative => Ok(Some(Observation::VerifiedRejection)),
        _ => Err(ContractError::UnverifiedStatus),
    }
}

/// Validate a read-only ERP snapshot before it can participate in reconciliation.
/// Exact duplicate event replays collapse; conflicting duplicates or broken reversal
/// links fail closed.
pub fn validate_erp_snapshot(
    expected_scope: CompanyScope,
    now_unix_s: u64,
    snapshot: &ErpSnapshot,
) -> Result<Vec<ErpEvent>, ContractError> {
    if snapshot.scope != expected_scope {
        return Err(ContractError::WrongScope);
    }
    if !snapshot.complete {
        return Err(ContractError::IncompleteSnapshot);
    }
    if snapshot.fresh_until_unix_s <= snapshot.as_of_unix_s
        || now_unix_s < snapshot.as_of_unix_s
        || now_unix_s > snapshot.fresh_until_unix_s
    {
        return Err(ContractError::StaleSnapshot);
    }
    let mut by_id = BTreeMap::<ResourceId, ErpEvent>::new();
    let mut ordered = Vec::new();
    let mut remaining = BTreeMap::<ResourceId, Money>::new();
    let mut last_sequence = 0_u64;
    for event in &snapshot.events {
        if event.sequence == 0 {
            return Err(ContractError::SequenceRegression);
        }
        if let Some(prior) = by_id.get(&event.external_id) {
            if prior != event {
                return Err(ContractError::DuplicateConflict);
            }
            continue;
        }
        if event.sequence <= last_sequence {
            return Err(ContractError::SequenceRegression);
        }
        // The adapter normalizes all event magnitudes to positive amounts;
        // reversal direction comes from the event kind, not a signed value.
        if event.amount.amount.decimal() <= 0.into() {
            return Err(ContractError::InvalidAmount);
        }
        if event.kind == ErpEventKind::Reversal {
            let Some(target) = event.reverses else {
                return Err(ContractError::InvalidReversal);
            };
            let Some(original) = by_id.get(&target) else {
                return Err(ContractError::InvalidReversal);
            };
            if original.kind == ErpEventKind::Reversal
                || original.amount.currency != event.amount.currency
            {
                return Err(ContractError::InvalidReversal);
            }
            let balance = remaining
                .get_mut(&target)
                .ok_or(ContractError::InvalidReversal)?;
            if event.amount.amount.decimal() > balance.amount.decimal() {
                return Err(ContractError::InvalidReversal);
            }
            *balance = balance
                .checked_sub(event.amount)
                .map_err(|_| ContractError::InvalidReversal)?;
        } else {
            if event.reverses.is_some() {
                return Err(ContractError::InvalidReversal);
            }
            remaining.insert(event.external_id, event.amount);
        }
        last_sequence = event.sequence;
        by_id.insert(event.external_id, *event);
        ordered.push(*event);
    }
    Ok(ordered)
}
