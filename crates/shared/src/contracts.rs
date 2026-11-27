use crate::{http_contract::IdempotencyKey, ids::*};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::future::Future;

/// Nonnegative PostgreSQL BIGINT capped at the largest exactly representable JavaScript integer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i64")]
pub struct Version(i64);
impl TryFrom<i64> for Version {
    type Error = &'static str;
    fn try_from(n: i64) -> Result<Self, Self::Error> {
        if !(0..=9_007_199_254_740_991).contains(&n) {
            Err("version outside safe wire range")
        } else {
            Ok(Self(n))
        }
    }
}
impl From<Version> for i64 {
    fn from(v: Version) -> Self {
        v.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Principal {
    User(ActorId),
    Worker(ActorId),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandContext {
    pub command: CommandId,
    pub correlation: CorrelationId,
    pub actor: Principal,
    pub scope: ResourceScope,
    pub expected_version: Option<Version>,
    pub occurred_at: DateTime<Utc>,
}
/// Context is constructed by trusted server-side authentication, never deserialized from a client.
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DossierKind {
    Payment,
    Advance,
    Settlement,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Outcome {
    Pass,
    Fail,
    Unknown,
    Error,
    NotApplicable,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Severity {
    Block,
    Warning,
    Info,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub correlation_id: CorrelationId,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub field_path: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Read,
    CreateDraft,
    EditDraft,
    UploadDocument,
    RequestPreflight,
    Seal,
    Export,
    Handoff,
    BeginReview,
    AcceptReview,
    ReturnReview,
    RequestWaiver,
    ApproveWaiver,
    PublishPolicy,
    VerifyReference,
    ReadAudit,
    ManageMembership,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorizationRequest {
    pub context: CommandContext,
    pub resource: ResourceId,
    pub resource_scope: ResourceScope,
    pub action: Action,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationDecision {
    Allow,
    Deny,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortError {
    Unavailable,
    Conflict,
    ScopeMismatch,
    InvalidInput,
}

/// Adapter evaluates current grants and SoD using the *same* transaction as the command.
pub trait AuthorizationPort<Tx>: Send + Sync {
    fn authorize<'a>(
        &'a self,
        tx: &'a mut Tx,
        request: &'a AuthorizationRequest,
    ) -> impl Future<Output = Result<AuthorizationDecision, PortError>> + Send + 'a;
}

/// Mandatory scope guard; never interprets scope agreement as authorization.
pub fn check_scope(request: &AuthorizationRequest) -> Result<(), PortError> {
    if request.context.scope != request.resource_scope {
        return Err(PortError::ScopeMismatch);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditAction {
    Create,
    Revise,
    Verify,
    Transition,
    Review,
    Waiver,
    Publish,
    Import,
    Access,
    Export,
    Reconcile,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEvent {
    pub context: CommandContext,
    pub resource: ResourceId,
    pub resource_scope: ResourceScope,
    pub action: AuditAction,
    pub revision: Option<RevisionId>,
    // Only IDs, hashes and enumerated actions here; no raw document text, tokens or bank details.
    pub evidence_ids: Vec<EvidenceId>,
}
/// Append inside the caller's transaction; no separate connection or eventual-only audit.
pub trait AuditPort<Tx>: Send + Sync {
    fn append<'a>(
        &'a self,
        tx: &'a mut Tx,
        event: &'a AuditEvent,
    ) -> impl Future<Output = Result<(), PortError>> + Send + 'a;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplayReceipt {
    /// Immutable receipt/result reference, not a pointer to mutable current state.
    pub result: ResourceId,
    pub version: Option<Version>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdempotencyDecision {
    Fresh,
    Replay(ReplayReceipt),
    Conflict,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdempotencyRequest {
    pub context: CommandContext,
    /// Registered server-side OpenAPI operation ID, never accepted from user body.
    pub operation_id: &'static str,
    pub target: Option<ResourceId>,
    pub key: IdempotencyKey,
    /// Hash canonical validated payload, target and expected version; never raw untrusted JSON.
    pub payload_hash: [u8; 32],
}
/// Scope keys by authenticated actor, tenant/company and operation.
/// Recheck current authorization before replay; reserve/check and store receipt atomically.
pub trait IdempotencyPort<Tx>: Send + Sync {
    fn check<'a>(
        &'a self,
        tx: &'a mut Tx,
        request: &'a IdempotencyRequest,
    ) -> impl Future<Output = Result<IdempotencyDecision, PortError>> + Send + 'a;
    fn record<'a>(
        &'a self,
        tx: &'a mut Tx,
        request: &'a IdempotencyRequest,
        receipt: ReplayReceipt,
    ) -> impl Future<Output = Result<(), PortError>> + Send + 'a;
}
