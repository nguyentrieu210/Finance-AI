//! E17 extractive guidance and validation of proposed citations.
//! Only use hits from the authenticated, policy-scoped retrieval adapter.
//! The validator checks citation provenance, not the truth of generated prose.
use crate::repository::Hit;
use finance_shared::upload_contract::Sha256Digest;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidEvidence,
    WrongPolicy,
    DuplicateCitation,
    UnknownCitation,
    InvalidQuote,
    TooManyItems,
}

/// Untrusted proposed citation. Source metadata is resolved from retrieved
/// evidence, never accepted from the proposal.
pub struct ProposedCitation<'a> {
    pub chunk_id: Uuid,
    pub quote: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Citation {
    pub chunk_id: Uuid,
    pub policy_id: Uuid,
    pub clause_id: Uuid,
    pub source_digest: String,
    pub source_page: i32,
    pub quote: String,
}

/// Guidance is not a rule result, approval, policy publication or decision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Guidance {
    pub advisory_only: bool,
    pub citations: Vec<Citation>,
}

fn evidence_index(
    policy: Uuid,
    hits: &[Hit],
) -> Result<BTreeMap<Uuid, &Hit>, Error> {
    if policy.is_nil() {
        return Err(Error::WrongPolicy);
    }
    if hits.len() > 20 {
        return Err(Error::TooManyItems);
    }
    let mut index = BTreeMap::new();
    for hit in hits {
        if hit.policy_id != policy {
            return Err(Error::WrongPolicy);
        }
        if hit.chunk_id.is_nil()
            || hit.clause_id.is_nil()
            || hit.source_page < 1
            || hit.excerpt.trim().is_empty()
            || hit.excerpt.len() > 8192
            || Sha256Digest::parse(&hit.source_digest).is_err()
            || !hit.distance.is_finite()
            || index.insert(hit.chunk_id, hit).is_some()
        {
            return Err(Error::InvalidEvidence);
        }
    }
    Ok(index)
}

pub fn validate_citations(
    policy: Uuid,
    hits: &[Hit],
    proposals: &[ProposedCitation<'_>],
) -> Result<Guidance, Error> {
    let index = evidence_index(policy, hits)?;
    if proposals.len() > 20 {
        return Err(Error::TooManyItems);
    }
    let mut seen = BTreeSet::new();
    let mut citations = Vec::with_capacity(proposals.len());
    for item in proposals {
        if !seen.insert(item.chunk_id) {
            return Err(Error::DuplicateCitation);
        }
        let hit = index.get(&item.chunk_id).ok_or(Error::UnknownCitation)?;
        if item.quote.trim().is_empty()
            || item.quote.len() > 8192
            || !hit.excerpt.contains(item.quote)
        {
            return Err(Error::InvalidQuote);
        }
        citations.push(Citation {
            chunk_id: hit.chunk_id,
            policy_id: hit.policy_id,
            clause_id: hit.clause_id,
            source_digest: hit.source_digest.clone(),
            source_page: hit.source_page,
            quote: item.quote.to_owned(),
        });
    }
    Ok(Guidance {
        advisory_only: true,
        citations,
    })
}

/// Deterministic extractive answer: no model, paraphrasing or invented text.
pub fn extractive(policy: Uuid, hits: &[Hit]) -> Result<Guidance, Error> {
    let proposals: Vec<_> = hits
        .iter()
        .map(|hit| ProposedCitation {
            chunk_id: hit.chunk_id,
            quote: &hit.excerpt,
        })
        .collect();
    validate_citations(policy, hits, &proposals)
}
