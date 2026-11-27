#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
use finance_shared::{
    ids::{ActorId, ResourceId, RevisionId},
    upload_contract::Sha256Digest,
};

pub struct Finding {
    pub id: ResourceId,
    pub revision: RevisionId,
    pub inputs: Sha256Digest,
    pub blocking: bool,
    pub waivable: bool,
    pub critical_unknown_or_error: bool,
}

pub struct Waiver {
    pub finding: ResourceId,
    pub revision: RevisionId,
    pub inputs: Sha256Digest,
    pub requester: ActorId,
    pub approver: ActorId,
    pub expires_at: i64,
    pub revoked: bool,
    pub authorized: bool,
}

pub fn blocks(finding: &Finding, waiver: Option<&Waiver>, now: i64) -> bool {
    if !finding.blocking {
        return false;
    }
    if !finding.waivable || finding.critical_unknown_or_error {
        return true;
    }
    !waiver.is_some_and(|w| {
        w.authorized
            && !w.revoked
            && w.requester != w.approver
            && w.expires_at > now
            && w.finding == finding.id
            && w.revision == finding.revision
            && w.inputs == finding.inputs
    })
}
