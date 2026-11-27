#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum External {
    NotSubmitted,
    DispatchPending,
    Unknown,
    Acknowledged,
    Approved,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Observation {
    Timeout,
    ManualReceipt,
    VerifiedApproval,
    VerifiedRejection,
}

/// No status update from timeout or manual receipt implies approval.
/// Verified events require independent authorization and source validation.
pub fn observe(state: External, event: Observation, reconciled: bool) -> Option<External> {
    use External as S;
    use Observation as E;
    match (state, event) {
        (S::DispatchPending, E::Timeout) => Some(S::Unknown),
        (S::DispatchPending | S::Unknown, E::ManualReceipt) => Some(S::Acknowledged),
        (S::Acknowledged | S::Unknown, E::VerifiedApproval) if reconciled => Some(S::Approved),
        (S::Acknowledged | S::Unknown, E::VerifiedRejection) if reconciled => Some(S::Rejected),
        _ => None,
    }
}

pub fn may_release(state: External, reconciled_no_payment: bool) -> bool {
    matches!(state, External::NotSubmitted | External::Rejected) && reconciled_no_payment
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lost_ack_holds_reservation() {
        assert_eq!(observe(External::DispatchPending, Observation::Timeout, false), Some(External::Unknown));
        assert!(!may_release(External::Unknown, true));
        assert_eq!(observe(External::Unknown, Observation::ManualReceipt, false), Some(External::Acknowledged));
        assert_eq!(observe(External::Unknown, Observation::VerifiedApproval, false), None);
    }
}
