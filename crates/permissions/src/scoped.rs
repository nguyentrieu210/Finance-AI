//! Scoped capability policy. Inputs must be resolved by trusted repositories in the
//! same transaction as the resource mutation. Not yet wired into live commands.
use finance_shared::{
    contracts::{CommandContext, Principal},
    ids::{ActorId, CompanyScope, ResourceId, ResourceScope},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capability {
    ReadOwn,
    ReadAssigned,
    ReadDepartment,
    ReadCompany,
    WriteOwn,
    ReviewAssigned,
    ConfirmDepartment,
    ApproveAssigned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrantScope {
    Company,
    Department(ResourceId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Grant {
    pub company: CompanyScope,
    pub actor: ActorId,
    pub capability: Capability,
    pub scope: GrantScope,
    pub active: bool,
    pub valid_from: i64,
    pub valid_until: Option<i64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Read,
    WriteDraft,
    Review,
    ConfirmDepartment,
    Approve,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DossierFacts {
    pub company: CompanyScope,
    pub department: Option<ResourceId>,
    pub owner: ActorId,
    pub reviewer: Option<ActorId>,
    pub department_confirmer: Option<ActorId>,
    pub approver: Option<ActorId>,
    pub reviewed_by: Option<ActorId>,
    pub confirmed_by: Option<ActorId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopedDenial {
    Scope,
    InactiveMembership,
    Worker,
    SelfApproval,
    NotAssigned,
    MissingGrant,
}

/// Explicit allow only. Administrative membership is not a dossier capability.
/// No department inheritance is assumed. Scope facts, active memberships and time
/// are server-resolved; never construct these from request JSON.
pub struct Evaluation<'a> {
    pub context: &'a CommandContext,
    pub membership_active: bool,
    pub active_departments: &'a [ResourceId],
    pub grants: &'a [Grant],
    pub now: i64,
    pub dossier: DossierFacts,
}

impl Evaluation<'_> {
    fn has(&self, actor: ActorId, capability: Capability) -> bool {
        self.grants.iter().any(|grant| {
            grant.company == self.dossier.company
                && grant.actor == actor
                && grant.capability == capability
                && grant.active
                && grant.valid_from <= self.now
                && grant.valid_until.is_none_or(|end| self.now < end)
                && match grant.scope {
                    GrantScope::Company => !matches!(
                        capability,
                        Capability::ReadDepartment | Capability::ConfirmDepartment
                    ),
                    GrantScope::Department(id) => {
                        capability != Capability::ReadCompany
                            && self.dossier.department == Some(id)
                            && self.active_departments.contains(&id)
                    }
                }
        })
    }

    pub fn decide(&self, operation: Operation) -> Result<(), ScopedDenial> {
        let d = self.dossier;
        if self.context.scope != ResourceScope::Company(d.company) {
            return Err(ScopedDenial::Scope);
        }
        let Principal::User(actor) = self.context.actor else {
            return Err(ScopedDenial::Worker);
        };
        if !self.membership_active {
            return Err(ScopedDenial::InactiveMembership);
        }
        let has = |capability| self.has(actor, capability);
        let allowed = match operation {
            Operation::Read => {
                (actor == d.owner && has(Capability::ReadOwn))
                    || ([d.reviewer, d.department_confirmer, d.approver].contains(&Some(actor))
                        && has(Capability::ReadAssigned))
                    || has(Capability::ReadDepartment)
                    || has(Capability::ReadCompany)
            }
            Operation::WriteDraft => actor == d.owner && has(Capability::WriteOwn),
            Operation::Review => {
                if actor == d.owner {
                    return Err(ScopedDenial::SelfApproval);
                }
                if d.reviewer != Some(actor) {
                    return Err(ScopedDenial::NotAssigned);
                }
                has(Capability::ReviewAssigned)
            }
            Operation::ConfirmDepartment => {
                if actor == d.owner {
                    return Err(ScopedDenial::SelfApproval);
                }
                if d.department_confirmer != Some(actor) {
                    return Err(ScopedDenial::NotAssigned);
                }
                has(Capability::ConfirmDepartment)
            }
            Operation::Approve => {
                if actor == d.owner || d.reviewed_by == Some(actor) || d.confirmed_by == Some(actor)
                {
                    return Err(ScopedDenial::SelfApproval);
                }
                if d.approver != Some(actor) {
                    return Err(ScopedDenial::NotAssigned);
                }
                has(Capability::ApproveAssigned)
            }
        };
        if allowed {
            Ok(())
        } else {
            Err(ScopedDenial::MissingGrant)
        }
    }
}

/// Role definitions are data, not authority by themselves. The trusted adapter must
/// load the published version and its assignments in the command transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleDefinition {
    pub company: CompanyScope,
    pub id: ResourceId,
    pub active: bool,
    pub capabilities: Vec<Capability>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RoleAssignment {
    pub company: CompanyScope,
    pub actor: ActorId,
    pub role_id: ResourceId,
    pub scope: GrantScope,
    pub active: bool,
    pub valid_from: i64,
    pub valid_until: Option<i64>,
}

/// Resolve many roles into the capabilities in their individual assignment scopes.
/// Duplicate role definitions are rejected as ambiguous; no implicit company grant
/// or hierarchy inheritance is generated from a department assignment.
pub fn resolve_grants(
    company: CompanyScope,
    actor: ActorId,
    definitions: &[RoleDefinition],
    assignments: &[RoleAssignment],
) -> Vec<Grant> {
    let mut grants = Vec::new();
    for assignment in assignments {
        if assignment.company != company
            || assignment.actor != actor
            || !assignment.active
            || assignment
                .valid_until
                .is_some_and(|end| end <= assignment.valid_from)
        {
            continue;
        }
        let mut matching = definitions
            .iter()
            .filter(|role| role.company == company && role.id == assignment.role_id);
        let Some(role) = matching.next() else {
            continue;
        };
        if !role.active || matching.next().is_some() {
            continue;
        }
        for capability in &role.capabilities {
            if matches!(
                (capability, assignment.scope),
                (Capability::ReadCompany, GrantScope::Department(_))
                    | (
                        Capability::ReadDepartment | Capability::ConfirmDepartment,
                        GrantScope::Company
                    )
            ) {
                continue;
            }
            let grant = Grant {
                company,
                actor,
                capability: *capability,
                scope: assignment.scope,
                active: true,
                valid_from: assignment.valid_from,
                valid_until: assignment.valid_until,
            };
            if !grants.contains(&grant) {
                grants.push(grant);
            }
        }
    }
    grants
}
