//! Offline comparison of already-authorized anonymized fact snapshots.
//! No policy publication, live reads, waivers, or writes occur here.
use crate::{
    Outcome,
    compiler::{self, Compiled, Fact},
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidComparison,
    TooManySnapshots,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Distribution {
    /// Rows/columns ordered PASS, FAIL, UNKNOWN, ERROR, NOT_APPLICABLE.
    pub transitions: [[u32; 5]; 5],
    pub evaluated: u32,
}

fn index(outcome: Outcome) -> usize {
    match outcome {
        Outcome::Pass => 0,
        Outcome::Fail => 1,
        Outcome::Unknown => 2,
        Outcome::Error => 3,
        Outcome::NotApplicable => 4,
    }
}

/// Both rules must have the same stable identifier and distinct versions.
/// The caller must enforce authorization, anonymization and snapshot provenance.
pub fn compare(
    baseline: &Compiled,
    candidate: &Compiled,
    snapshots: &[BTreeMap<String, Fact>],
) -> Result<Distribution, Error> {
    if baseline.rule_id() != candidate.rule_id() || baseline.version() >= candidate.version() {
        return Err(Error::InvalidComparison);
    }
    if snapshots.len() > 5000 || snapshots.iter().any(|s| s.len() > 5000) {
        return Err(Error::TooManySnapshots);
    }
    let mut result = Distribution {
        transitions: [[0; 5]; 5],
        evaluated: 0,
    };
    for snapshot in snapshots {
        let before = index(compiler::evaluate(baseline, snapshot).outcome);
        let after = index(compiler::evaluate(candidate, snapshot).outcome);
        result.transitions[before][after] += 1;
        result.evaluated += 1;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(version: u64, condition: bool) -> Compiled {
        let input = json!({
            "rule_id": "R02",
            "version": version,
            "condition": {"literal": condition, "value_type": "boolean"},
            "severity": "BLOCK",
            "waivable": false,
            "clause_id": "00000000-0000-0000-0000-000000000001"
        });
        compiler::compile(&serde_json::to_vec(&input).unwrap(), &BTreeMap::new()).unwrap()
    }

    #[test]
    fn compares_versions_without_mutating_facts() {
        let old = rule(1, true);
        let new = rule(2, false);
        let snapshots = [BTreeMap::new(), BTreeMap::new()];
        let result = compare(&old, &new, &snapshots).unwrap();
        assert_eq!(result.evaluated, 2);
        assert_eq!(result.transitions[0][1], 2);
        assert_eq!(result.transitions[1][0], 0);
        assert_eq!(
            compare(&new, &old, &snapshots),
            Err(Error::InvalidComparison)
        );
        assert!(snapshots.iter().all(BTreeMap::is_empty));
    }
}
