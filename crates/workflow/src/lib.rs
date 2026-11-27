#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Draft,
    Processing,
    NeedsInput,
    Reviewable,
    Ready,
    Exported,
    HandedOff,
    AccountingReview,
    Accepted,
    Returned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    Process,
    ProcessComplete,
    Ready,
    Export,
    Handoff,
    BeginReview,
    Accept,
    Return,
    Reopen,
}

#[derive(Default)]
pub struct Guards {
    pub authorized: bool,
    pub current_revision: bool,
    pub clean_docs: bool,
    pub worker_fenced: bool,
    pub complete: bool,
    pub verified: bool,
    pub rules_pass: bool,
    pub reserved: bool,
    pub snapshot_current: bool,
    pub artifact_verified: bool,
    pub external_receipt: bool,
    pub independent_accountant: bool,
    pub critical_confirmed: bool,
    pub required_external_evidence: bool,
    pub return_reason: bool,
    pub reconciled: bool,
}

/// Pure transition proposal. Real authorization, locking, sealing and audit
/// must occur atomically in repository layer; booleans are not proof.
pub fn transition(state: State, command: Command, g: &Guards) -> Option<State> {
    use Command as C;
    use State as S;
    if !g.authorized || !g.current_revision {
        return None;
    }
    match (state, command) {
        (S::Draft | S::NeedsInput, C::Process) if g.clean_docs => Some(S::Processing),
        (S::Processing, C::ProcessComplete) if g.worker_fenced => Some(if g.complete {
            S::Reviewable
        } else {
            S::NeedsInput
        }),
        (S::Reviewable, C::Ready)
            if g.complete && g.verified && g.rules_pass && g.reserved && g.snapshot_current =>
        {
            Some(S::Ready)
        }
        (S::Ready, C::Export) if g.artifact_verified && g.snapshot_current => Some(S::Exported),
        (S::Exported, C::Handoff)
            if g.external_receipt && g.reserved && g.snapshot_current && g.rules_pass =>
        {
            Some(S::HandedOff)
        }
        (S::HandedOff, C::BeginReview) if g.independent_accountant => Some(S::AccountingReview),
        (S::AccountingReview, C::Accept)
            if g.independent_accountant
                && g.critical_confirmed
                && g.required_external_evidence
                && g.snapshot_current
                && g.rules_pass =>
        {
            Some(S::Accepted)
        }
        (S::AccountingReview, C::Return)
            if g.independent_accountant && g.return_reason && g.reconciled =>
        {
            Some(S::Returned)
        }
        (S::Returned, C::Reopen) if g.reconciled => Some(S::Draft),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_direct_approval_or_stale_ready() {
        let mut g = Guards {
            authorized: true,
            current_revision: true,
            complete: true,
            verified: true,
            rules_pass: true,
            reserved: true,
            snapshot_current: true,
            ..Default::default()
        };
        assert_eq!(transition(State::Draft, Command::Accept, &g), None);
        assert_eq!(
            transition(State::Reviewable, Command::Ready, &g),
            Some(State::Ready)
        );
        g.snapshot_current = false;
        assert_eq!(transition(State::Reviewable, Command::Ready, &g), None);
        assert_eq!(
            transition(State::AccountingReview, Command::Accept, &g),
            None
        );
    }
}

pub mod analytics;
pub mod checks;
pub mod draft;
