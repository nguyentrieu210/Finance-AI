//! Bound rule-set evaluation for a trusted, immutable dossier/policy snapshot.
//! This module emits diagnostic outcomes, never a READY or approval transition.
use crate::{
    Outcome,
    compiler::{self, Compiled, Fact, Severity},
};
use finance_shared::{
    ids::{CompanyScope, DossierId, EvidenceId, ResourceId, RevisionId},
    upload_contract::Sha256Digest,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub scope: CompanyScope,
    pub dossier: DossierId,
    pub revision: RevisionId,
    pub version: u64,
    pub policy: ResourceId,
    pub digest: Sha256Digest,
}

pub struct BoundFact {
    pub snapshot: Snapshot,
    pub path: String,
    pub fact: Fact,
}

pub struct BoundRule<'a> {
    pub policy: ResourceId,
    pub digest: Sha256Digest,
    pub compiled: &'a Compiled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    StaleSnapshot,
    WrongPolicy,
    DuplicateFact,
    DuplicateRule,
    Bounds,
    MissingRuleSet,
}

pub struct ResultRow {
    pub rule_id: String,
    pub version: u64,
    pub severity: Severity,
    pub outcome: Outcome,
    pub evidence: Vec<EvidenceId>,
    pub evaluated_refs: Vec<String>,
}

pub struct Report {
    pub snapshot: Snapshot,
    pub results: Vec<ResultRow>,
    pub blocked: bool,
}

pub fn evaluate(
    snapshot: Snapshot,
    current: Snapshot,
    rules: &[BoundRule<'_>],
    facts: &[BoundFact],
) -> Result<Report, Error> {
    if snapshot != current {
        return Err(Error::StaleSnapshot);
    }
    if rules.is_empty() {
        return Err(Error::MissingRuleSet);
    }
    if rules.len() > 1000 || facts.len() > 5000 || snapshot.version > 9_007_199_254_740_991 {
        return Err(Error::Bounds);
    }
    let mut bound = BTreeMap::new();
    for fact in facts {
        if fact.snapshot != snapshot {
            return Err(Error::StaleSnapshot);
        }
        if fact.path.is_empty() || fact.path.len() > 128 || fact.fact.evidence.len() > 64 {
            return Err(Error::Bounds);
        }
        if bound.insert(fact.path.clone(), fact.fact.clone()).is_some() {
            return Err(Error::DuplicateFact);
        }
    }
    let mut seen = BTreeSet::new();
    let mut results = Vec::with_capacity(rules.len());
    let mut blocked = false;
    for rule in rules {
        if rule.policy != snapshot.policy {
            return Err(Error::WrongPolicy);
        }
        let id = rule.compiled.rule_id();
        if !seen.insert(id) {
            return Err(Error::DuplicateRule);
        }
        let evaluated = compiler::evaluate(rule.compiled, &bound);
        let severity = rule.compiled.severity();
        blocked |= crate::blocks(evaluated.outcome, severity == Severity::Block);
        results.push(ResultRow {
            rule_id: id.to_owned(),
            version: rule.compiled.version(),
            severity,
            outcome: evaluated.outcome,
            evidence: evaluated.evidence,
            evaluated_refs: evaluated.evaluated_refs,
        });
    }
    results.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
    Ok(Report {
        snapshot,
        results,
        blocked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_shared::ids::{CompanyId, TenantId};
    use uuid::Uuid;
    #[test]
    fn rejects_cross_revision_and_missing_rules() {
        let snapshot = Snapshot {
            scope: CompanyScope {
                tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
                company: CompanyId::try_from(Uuid::from_u128(2)).unwrap(),
            },
            dossier: DossierId::try_from(Uuid::from_u128(3)).unwrap(),
            revision: RevisionId::try_from(Uuid::from_u128(4)).unwrap(),
            version: 1,
            policy: ResourceId::try_from(Uuid::from_u128(5)).unwrap(),
            digest: Sha256Digest::parse(&"a".repeat(64)).unwrap(),
        };
        assert!(matches!(
            evaluate(
                snapshot,
                Snapshot {
                    version: 2,
                    ..snapshot
                },
                &[],
                &[]
            ),
            Err(Error::StaleSnapshot)
        ));
        assert!(matches!(
            evaluate(snapshot, snapshot, &[], &[]),
            Err(Error::MissingRuleSet)
        ));
    }
}
