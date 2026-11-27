#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
use finance_shared::ids::{CompanyScope, ResourceId};

pub struct Clause<'a> {
    pub scope: CompanyScope,
    pub version: ResourceId,
    pub id: ResourceId,
    pub approved_current: bool,
    pub text: &'a str,
}

/// Caller supplies the effective approved policy version. No cross-scope
/// retrieval or model execution. Return references, never fabricated citations.
pub fn retrieve<'a>(clauses: &'a [Clause<'a>], scope: CompanyScope, version: ResourceId, query: &str) -> Vec<&'a Clause<'a>> {
    if query.trim().is_empty() || query.len() > 256 || clauses.len() > 10_000 {
        return vec![];
    }
    let query = query.to_lowercase();
    clauses.iter().filter(|c| c.scope == scope && c.version == version && c.approved_current && c.text.len() <= 8192 && c.text.to_lowercase().contains(&query)).take(20).collect()
}
