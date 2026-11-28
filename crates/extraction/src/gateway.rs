//! E08 preflight for an isolated AI adapter. No provider transport or credentials.
use finance_shared::{ids::CompanyScope, upload_contract::Sha256Digest};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Disabled,
    WrongScope,
    UnapprovedProvider,
    InvalidLimits,
    UnsafeRequest,
}

pub struct Approval<'a> {
    pub scope: CompanyScope,
    pub provider: &'a str,
    pub region: &'a str,
    pub model: &'a str,
    pub prompt: Sha256Digest,
    pub policy_current: bool,
    pub synthetic_only: bool,
    pub max_input_bytes: usize,
    pub max_output_tokens: u32,
    pub timeout_seconds: u32,
}

pub struct Request<'a> {
    pub scope: CompanyScope,
    pub provider: &'a str,
    pub region: &'a str,
    pub model: &'a str,
    pub prompt: Sha256Digest,
    pub input_bytes: usize,
    pub output_tokens: u32,
    pub timeout_seconds: u32,
    pub synthetic: bool,
    pub minimized: bool,
    pub tools_enabled: bool,
}

/// Private proof is only an in-process synthetic capability decision. No
/// production egress or real documents are allowed by this prototype.
pub struct Permit {
    _private: (),
}

pub fn authorize(approval: Option<&Approval<'_>>, request: &Request<'_>) -> Result<Permit, Error> {
    let approval = approval.ok_or(Error::Disabled)?;
    if !approval.policy_current || !approval.synthetic_only {
        return Err(Error::Disabled);
    }
    if approval.scope != request.scope {
        return Err(Error::WrongScope);
    }
    if approval.provider != request.provider
        || approval.region != request.region
        || approval.model != request.model
        || approval.prompt != request.prompt
        || [request.provider, request.region, request.model]
            .iter()
            .any(|v| v.is_empty() || v.len() > 128 || v.chars().any(char::is_control))
    {
        return Err(Error::UnapprovedProvider);
    }
    if !request.synthetic || !request.minimized || request.tools_enabled {
        return Err(Error::UnsafeRequest);
    }
    if !(1..=262_144).contains(&approval.max_input_bytes)
        || !(1..=4096).contains(&approval.max_output_tokens)
        || !(1..=60).contains(&approval.timeout_seconds)
        || !(1..=approval.max_input_bytes).contains(&request.input_bytes)
        || !(1..=approval.max_output_tokens).contains(&request.output_tokens)
        || !(1..=approval.timeout_seconds).contains(&request.timeout_seconds)
    {
        return Err(Error::InvalidLimits);
    }
    Ok(Permit { _private: () })
}
