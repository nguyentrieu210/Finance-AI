//! Synthetic immutable external observation reducer. No DO/ERP network or credentials.
use crate::{External, Observation, observe};
use finance_shared::{ids::{CompanyScope, ResourceId, RevisionId}, upload_contract::Sha256Digest};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    WrongBinding,
    ReplayConflict,
    StaleSequence,
    InvalidTransition,
    TooManyEvents,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Apply {
    Applied,
    Replayed,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Event {
    pub scope: CompanyScope,
    pub handoff: ResourceId,
    pub revision: RevisionId,
    pub external_id: ResourceId,
    pub sequence: u64,
    pub source_digest: Sha256Digest,
    pub observation: Observation,
    pub independently_verified: bool,
}

pub struct Reconciler {
    scope: CompanyScope,
    handoff: ResourceId,
    revision: RevisionId,
    state: External,
    high_watermark: u64,
    events: BTreeMap<ResourceId, Event>,
}

impl Reconciler {
    pub fn new(scope: CompanyScope, handoff: ResourceId, revision: RevisionId) -> Self {
        Self {
            scope, handoff, revision,
            state: External::DispatchPending,
            high_watermark: 0,
            events: BTreeMap::new(),
        }
    }

    pub fn state(&self) -> External { self.state }

    pub fn apply(&mut self, event: Event) -> Result<Apply, Error> {
        if event.scope != self.scope || event.handoff != self.handoff || event.revision != self.revision {
            return Err(Error::WrongBinding);
        }
        if let Some(prior) = self.events.get(&event.external_id) {
            return if *prior == event { Ok(Apply::Replayed) } else { Err(Error::ReplayConflict) };
        }
        if event.sequence <= self.high_watermark {
            return Err(Error::StaleSequence);
        }
        if self.events.len() >= 10_000 {
            return Err(Error::TooManyEvents);
        }
        // Manual receipt must also be independently verified before acknowledgement.
        if event.observation != Observation::Timeout && !event.independently_verified {
            return Err(Error::InvalidTransition);
        }
        let next = observe(self.state, event.observation, event.independently_verified).ok_or(Error::InvalidTransition)?;
        self.state = next;
        self.high_watermark = event.sequence;
        self.events.insert(event.external_id, event);
        Ok(Apply::Applied)
    }
}
