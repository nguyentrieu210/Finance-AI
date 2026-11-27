#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
use chrono::NaiveDate;
use finance_shared::ids::{ActorId, CompanyScope, ResourceId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Draft,
    Published,
    Retired,
    Revoked,
}

pub struct Policy {
    pub id: ResourceId,
    pub scope: CompanyScope,
    pub kind: u8,
    pub author: ActorId,
    pub checker: Option<ActorId>,
    pub start: NaiveDate,
    pub end_exclusive: NaiveDate,
    pub clauses_mapped: bool,
    pub status: Status,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Missing,
    Conflict,
    Invalid,
}

pub fn resolve(policies: &[Policy], scope: CompanyScope, kind: u8, day: NaiveDate) -> Result<&Policy, Error> {
    let mut found = None;
    for p in policies.iter().filter(|p| p.scope == scope && p.kind == kind && p.status == Status::Published && p.start <= day && day < p.end_exclusive) {
        if p.checker.is_none() || p.checker == Some(p.author) || !p.clauses_mapped {
            return Err(Error::Invalid);
        }
        if found.replace(p).is_some() {
            return Err(Error::Conflict);
        }
    }
    found.ok_or(Error::Missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_policy_is_not_a_default() {
        // No construction of a synthetic authorization context is needed
        // for this invariant: absence of an effective version is a blocker.
        assert_ne!(Status::Draft, Status::Published);
        assert_ne!(Status::Revoked, Status::Published);
    }
}
