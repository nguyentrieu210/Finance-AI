use finance_permissions::*;
use finance_shared::{
    contracts::{Action, CommandContext, Principal},
    ids::*,
};
use uuid::Uuid;
fn actor(n: u128) -> ActorId {
    ActorId::try_from(Uuid::from_u128(n)).unwrap()
}
fn scope(n: u128) -> CompanyScope {
    CompanyScope {
        tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
        company: CompanyId::try_from(Uuid::from_u128(n)).unwrap(),
    }
}
fn ctx(p: Principal) -> CommandContext {
    CommandContext {
        command: CommandId::try_from(Uuid::from_u128(40)).unwrap(),
        correlation: CorrelationId::try_from(Uuid::from_u128(41)).unwrap(),
        actor: p,
        scope: ResourceScope::Company(scope(10)),
        expected_version: None,
        occurred_at: "2026-01-01T00:00:00Z".parse().unwrap(),
    }
}
#[test]
fn owner_assignment_and_scope() {
    let c = ctx(Principal::User(actor(2)));
    let own = ResourceFacts::Dossier {
        owner: actor(2),
        reviewer: Some(actor(3)),
    };
    let other = ResourceFacts::Dossier {
        owner: actor(3),
        reviewer: Some(actor(2)),
    };
    assert_eq!(
        decide(&c, scope(10), &[Role::Requester], Action::EditDraft, own),
        Ok(())
    );
    assert_eq!(
        decide(&c, scope(11), &[Role::Requester], Action::EditDraft, own),
        Err(Denial::Scope)
    );
    assert_eq!(
        decide(&c, scope(10), &[Role::Requester], Action::EditDraft, other),
        Err(Denial::NotOwner)
    );
    assert_eq!(
        decide(
            &c,
            scope(10),
            &[Role::Accountant],
            Action::AcceptReview,
            other
        ),
        Ok(())
    );
    assert_eq!(
        decide(
            &c,
            scope(10),
            &[Role::Accountant],
            Action::AcceptReview,
            ResourceFacts::Dossier {
                owner: actor(3),
                reviewer: None
            }
        ),
        Err(Denial::NotAssigned)
    );
}
#[test]
fn dual_roles_cannot_self_approve() {
    let c = ctx(Principal::User(actor(2)));
    let roles = [
        Role::Requester,
        Role::Accountant,
        Role::FinanceApprover,
        Role::PolicyAdmin,
        Role::OrgAdmin,
    ];
    assert_eq!(
        decide(
            &c,
            scope(10),
            &roles,
            Action::AcceptReview,
            ResourceFacts::Dossier {
                owner: actor(2),
                reviewer: Some(actor(2))
            }
        ),
        Err(Denial::SelfApproval)
    );
    assert_eq!(
        decide(
            &c,
            scope(10),
            &roles,
            Action::ApproveWaiver,
            ResourceFacts::Waiver {
                requester: actor(3),
                dossier_owner: actor(2)
            }
        ),
        Err(Denial::SelfApproval)
    );
    assert_eq!(
        decide(
            &c,
            scope(10),
            &roles,
            Action::PublishPolicy,
            ResourceFacts::Policy { author: actor(2) }
        ),
        Err(Denial::SelfApproval)
    );
    assert_eq!(
        decide(
            &c,
            scope(10),
            &roles,
            Action::VerifyReference,
            ResourceFacts::Reference { author: actor(2) }
        ),
        Err(Denial::SelfApproval)
    );
    assert_eq!(
        decide(
            &c,
            scope(10),
            &roles,
            Action::ManageMembership,
            ResourceFacts::Organization
        ),
        Err(Denial::Unsupported)
    );
}
#[test]
fn worker_and_unknown_actions_denied() {
    let w = ctx(Principal::Worker(actor(2)));
    let c = ctx(Principal::User(actor(2)));
    assert_eq!(
        decide(
            &w,
            scope(10),
            &[Role::FinanceApprover],
            Action::PublishPolicy,
            ResourceFacts::Policy { author: actor(3) }
        ),
        Err(Denial::Worker)
    );
    assert_eq!(
        decide(
            &c,
            scope(10),
            &[Role::OrgAdmin],
            Action::ReadAudit,
            ResourceFacts::Audit
        ),
        Err(Denial::MissingGrant)
    );
    assert_eq!(
        decide(
            &c,
            scope(10),
            &[Role::Requester],
            Action::Export,
            ResourceFacts::Dossier {
                owner: actor(2),
                reviewer: None
            }
        ),
        Err(Denial::Unsupported)
    );
}
