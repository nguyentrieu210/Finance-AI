//! Scope-filtered rule coverage. Unknown or error never inflates automation.
use finance_shared::ids::{CompanyScope, DossierId, ResourceId};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Pass,
    Fail,
    Unknown,
    Error,
    NotApplicable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Observation {
    pub scope: CompanyScope,
    pub dossier: DossierId,
    pub criterion: ResourceId,
    pub outcome: Outcome,
    pub automatic: bool,
    pub evidenced: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Coverage {
    pub applicable: usize,
    pub automatic_evidenced: usize,
    pub unknown_or_error: usize,
    pub excluded_not_applicable: usize,
}

/// Conflicting duplicates fail instead of choosing an arbitrary result.
/// Caller supplies one authorized, pinned cohort and current observations.
pub fn calculate(scope: CompanyScope, rows: &[Observation]) -> Option<Coverage> {
    if rows.len() > 100_000 {
        return None;
    }
    let mut unique = BTreeMap::new();
    for row in rows.iter().filter(|r| r.scope == scope) {
        let key = (row.dossier, row.criterion);
        if unique.insert(key, row).is_some_and(|prior| prior != row) {
            return None;
        }
    }
    let mut result = Coverage::default();
    for row in unique.values() {
        if row.outcome == Outcome::NotApplicable && row.evidenced {
            result.excluded_not_applicable += 1;
            continue;
        }
        result.applicable += 1;
        match row.outcome {
            Outcome::Pass | Outcome::Fail if row.evidenced => {
                if row.automatic {
                    result.automatic_evidenced += 1;
                }
            }
            _ => result.unknown_or_error += 1,
        }
    }
    Some(result)
}
