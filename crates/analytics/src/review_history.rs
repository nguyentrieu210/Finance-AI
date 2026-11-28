//! E24 review KPI from pinned, complete review-decision histories.
//! Does not infer first-pass acceptance from a current dossier status.
use finance_shared::ids::{CompanyScope, DossierId, ResourceId};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CohortCase {
    pub scope: CompanyScope,
    pub dossier: DossierId,
    /// Number of review events in the complete history as of the pinned cutoff.
    pub event_count: u32,
    pub history_complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Returned,
    Reopened,
    Accepted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Event {
    pub scope: CompanyScope,
    pub dossier: DossierId,
    pub id: ResourceId,
    /// Contiguous sequence of review decisions, not general dossier versions.
    pub sequence: u32,
    pub occurred_unix_s: u64,
    pub decision: Decision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    TooManyRows,
    IncompleteHistory,
    ConflictingReplay,
    OutsideCohort,
    InvalidTransition,
    InvalidTime,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Counts {
    pub cases: usize,
    pub accepted: usize,
    pub first_pass_accepted: usize,
    pub returned_cases: usize,
    pub not_accepted: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ReviewState {
    Pending,
    Returned,
    Accepted,
}

/// Deduplicate exact event replay, reject incomplete or contradictory histories,
/// and isolate every count to the selected company. Caller supplies verified
/// review decisions and a complete cohort; this function grants no authorization.
pub fn calculate(
    scope: CompanyScope,
    cohort: &[CohortCase],
    events: &[Event],
    as_of_unix_s: u64,
) -> Result<Counts, Error> {
    if cohort.len() > 100_000 || events.len() > 500_000 {
        return Err(Error::TooManyRows);
    }
    let mut cases = BTreeMap::new();
    for row in cohort.iter().filter(|row| row.scope == scope) {
        if !row.history_complete {
            return Err(Error::IncompleteHistory);
        }
        if cases
            .insert(row.dossier, row.event_count)
            .is_some_and(|old| old != row.event_count)
        {
            return Err(Error::ConflictingReplay);
        }
    }
    let mut by_id = BTreeMap::new();
    let mut grouped = BTreeMap::<DossierId, BTreeMap<u32, &Event>>::new();
    for event in events.iter().filter(|event| event.scope == scope) {
        if event.occurred_unix_s > as_of_unix_s {
            return Err(Error::InvalidTime);
        }
        let count = cases.get(&event.dossier).ok_or(Error::OutsideCohort)?;
        if !(1..=*count).contains(&event.sequence) {
            return Err(Error::IncompleteHistory);
        }
        if let Some(old) = by_id.insert(event.id, event) {
            if old != event {
                return Err(Error::ConflictingReplay);
            }
        }
        if grouped
            .entry(event.dossier)
            .or_default()
            .insert(event.sequence, event)
            .is_some_and(|old| old != event)
        {
            return Err(Error::ConflictingReplay);
        }
    }
    let mut result = Counts {
        cases: cases.len(),
        ..Default::default()
    };
    for (dossier, count) in cases {
        let history = grouped.remove(&dossier).unwrap_or_default();
        if history.len() != count as usize {
            return Err(Error::IncompleteHistory);
        }
        let mut state = ReviewState::Pending;
        let mut returned = false;
        let mut previous_time = 0;
        for event in history.values() {
            if event.occurred_unix_s < previous_time {
                return Err(Error::InvalidTime);
            }
            previous_time = event.occurred_unix_s;
            state = match (state, event.decision) {
                (ReviewState::Pending, Decision::Returned) => {
                    returned = true;
                    ReviewState::Returned
                }
                (ReviewState::Returned, Decision::Reopened) => ReviewState::Pending,
                (ReviewState::Pending, Decision::Accepted) => ReviewState::Accepted,
                _ => return Err(Error::InvalidTransition),
            };
        }
        if returned {
            result.returned_cases += 1;
        }
        if state == ReviewState::Accepted {
            result.accepted += 1;
            if !returned {
                result.first_pass_accepted += 1;
            }
        } else {
            result.not_accepted += 1;
        }
    }
    Ok(result)
}
