#![forbid(unsafe_code)]
//! Internal synthetic policy candidate. Resource attributes must come from a trusted repository,
//! not HTTP fields. No financial action is enabled by this evaluator.
pub mod scoped;

use finance_shared::{
    contracts::{Action, CommandContext, Principal},
    ids::{ActorId, CompanyScope, ResourceScope},
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    OrgAdmin,
    Requester,
    Accountant,
    Auditor,
    FinanceApprover,
    PolicyAdmin,
}
impl Role {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ORG_ADMIN" => Self::OrgAdmin,
            "REQUESTER" => Self::Requester,
            "ACCOUNTANT" => Self::Accountant,
            "AUDITOR" => Self::Auditor,
            "FINANCE_APPROVER" => Self::FinanceApprover,
            "POLICY_ADMIN" => Self::PolicyAdmin,
            _ => return None,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denial {
    Scope,
    Worker,
    MissingGrant,
    NotOwner,
    NotAssigned,
    SelfApproval,
    Unsupported,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceFacts {
    Dossier {
        owner: ActorId,
        reviewer: Option<ActorId>,
    },
    Waiver {
        requester: ActorId,
        dossier_owner: ActorId,
    },
    Policy {
        author: ActorId,
    },
    Reference {
        author: ActorId,
    },
    Audit,
    Organization,
}
/// Conservative draft: only own requests; explicit reviewer assignment; maker/checker independent.
/// Delegation, authority amounts and additional policy actions not yet enabled.
pub fn decide(
    ctx: &CommandContext,
    scope: CompanyScope,
    roles: &[Role],
    action: Action,
    facts: ResourceFacts,
) -> Result<(), Denial> {
    if ctx.scope != ResourceScope::Company(scope) {
        return Err(Denial::Scope);
    }
    let Principal::User(actor) = ctx.actor else {
        return Err(Denial::Worker);
    };
    let has = |r| roles.contains(&r);
    let require = |r| {
        if has(r) {
            Ok(())
        } else {
            Err(Denial::MissingGrant)
        }
    };
    match (action, facts) {
        (Action::ReadAudit, ResourceFacts::Audit) => require(Role::Auditor),
        (
            Action::CreateDraft
            | Action::EditDraft
            | Action::UploadDocument
            | Action::RequestPreflight
            | Action::Handoff
            | Action::RequestWaiver,
            ResourceFacts::Dossier { owner, .. },
        ) => {
            require(Role::Requester)?;
            if actor != owner {
                return Err(Denial::NotOwner);
            }
            Ok(())
        }
        (
            Action::BeginReview | Action::AcceptReview | Action::ReturnReview,
            ResourceFacts::Dossier { owner, reviewer },
        ) => {
            require(Role::Accountant)?;
            if actor == owner {
                return Err(Denial::SelfApproval);
            }
            if reviewer != Some(actor) {
                return Err(Denial::NotAssigned);
            }
            Ok(())
        }
        (Action::Read, ResourceFacts::Dossier { owner, reviewer }) => {
            if (has(Role::Requester) && actor == owner)
                || (has(Role::Accountant) && reviewer == Some(actor))
            {
                Ok(())
            } else {
                Err(Denial::MissingGrant)
            }
        }
        (
            Action::ApproveWaiver,
            ResourceFacts::Waiver {
                requester,
                dossier_owner,
            },
        ) => {
            require(Role::FinanceApprover)?;
            if actor == requester || actor == dossier_owner {
                return Err(Denial::SelfApproval);
            }
            Ok(())
        }
        (Action::PublishPolicy, ResourceFacts::Policy { author }) => {
            require(Role::FinanceApprover)?;
            if actor == author {
                return Err(Denial::SelfApproval);
            }
            Ok(())
        }
        (Action::VerifyReference, ResourceFacts::Reference { author }) => {
            require(Role::FinanceApprover)?;
            if actor == author {
                return Err(Denial::SelfApproval);
            }
            Ok(())
        }
        // Membership grant administration, sealed transitions, export and unknown resource pairs are denied.
        _ => Err(Denial::Unsupported),
    }
}
#[derive(Debug)]
pub enum GrantError {
    Forbidden,
    Database(sqlx::Error),
}
impl From<sqlx::Error> for GrantError {
    fn from(e: sqlx::Error) -> Self {
        Self::Database(e)
    }
}
/// Lock current grants within the same READ COMMITTED transaction as effects.
/// Reject a different context from the scope actually set on the connection.
pub async fn lock_roles(
    tx: &mut Transaction<'_, Postgres>,
    ctx: &CommandContext,
) -> Result<Vec<Role>, GrantError> {
    let ResourceScope::Company(scope) = ctx.scope else {
        return Err(GrantError::Forbidden);
    };
    let Principal::User(actor) = ctx.actor else {
        return Err(GrantError::Forbidden);
    };
    let (t, c, a): (Uuid, Uuid, Uuid) = (scope.tenant.into(), scope.company.into(), actor.into());
    let matches:bool=sqlx::query_scalar("SELECT COALESCE(current_user='finance_runtime' AND current_setting('transaction_isolation')='read committed' AND finance.scope_tenant()=$1 AND finance.scope_company()=$2 AND finance.scope_actor()=$3,false)")
        .bind(t).bind(c).bind(a).fetch_one(&mut **tx).await?;
    if !matches {
        return Err(GrantError::Forbidden);
    }
    let roles: Vec<String> = sqlx::query_scalar("SELECT finance.lock_current_roles()")
        .fetch_one(&mut **tx)
        .await?;
    roles
        .iter()
        .map(|r| Role::parse(r).ok_or(GrantError::Forbidden))
        .collect()
}
pub async fn require_role(
    tx: &mut Transaction<'_, Postgres>,
    ctx: &CommandContext,
    role: Role,
) -> Result<(), GrantError> {
    if lock_roles(tx, ctx).await?.contains(&role) {
        Ok(())
    } else {
        Err(GrantError::Forbidden)
    }
}
