//! E24 active handling time over explicit, verified intervals.
//! Never infer work time from dossier creation, status, or elapsed wall time.
use finance_shared::ids::{CompanyScope, DossierId};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Interval {
    pub scope: CompanyScope,
    pub dossier: DossierId,
    pub start_unix_s: u64,
    pub end_unix_s: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CohortCase {
    pub scope: CompanyScope,
    pub dossier: DossierId,
    pub history_complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    IncompleteHistory,
    InvalidInterval,
    OutsideCohort,
    TooManyRows,
    Overflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandlingTime {
    pub dossier: DossierId,
    pub active_seconds: u64,
}

/// Caller provides authorized, pinned cohort membership and independently
/// verified interval history. Missing coverage blocks rather than reporting zero.
/// Overlap and exact replay are counted once per dossier.
pub fn calculate(
    scope: CompanyScope,
    cohort: &[CohortCase],
    intervals: &[Interval],
    as_of_unix_s: u64,
) -> Result<Vec<HandlingTime>, Error> {
    if cohort.len() > 100_000 || intervals.len() > 500_000 {
        return Err(Error::TooManyRows);
    }
    let mut groups = BTreeMap::<DossierId, Vec<(u64, u64)>>::new();
    for row in cohort.iter().filter(|row| row.scope == scope) {
        if !row.history_complete {
            return Err(Error::IncompleteHistory);
        }
        groups.entry(row.dossier).or_default();
    }
    for row in intervals.iter().filter(|row| row.scope == scope) {
        if row.start_unix_s >= row.end_unix_s || row.end_unix_s > as_of_unix_s {
            return Err(Error::InvalidInterval);
        }
        groups
            .get_mut(&row.dossier)
            .ok_or(Error::OutsideCohort)?
            .push((row.start_unix_s, row.end_unix_s));
    }
    groups
        .into_iter()
        .map(|(dossier, mut ranges)| {
            ranges.sort_unstable();
            let mut total = 0_u64;
            let mut current: Option<(u64, u64)> = None;
            for (start, end) in ranges {
                match current {
                    Some((left, right)) if start <= right => {
                        current = Some((left, right.max(end)));
                    }
                    Some((left, right)) => {
                        total = total.checked_add(right - left).ok_or(Error::Overflow)?;
                        current = Some((start, end));
                    }
                    None => current = Some((start, end)),
                }
            }
            if let Some((left, right)) = current {
                total = total.checked_add(right - left).ok_or(Error::Overflow)?;
            }
            Ok(HandlingTime {
                dossier,
                active_seconds: total,
            })
        })
        .collect()
}
