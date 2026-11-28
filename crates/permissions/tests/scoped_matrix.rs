use finance_permissions::scoped::*;
use finance_shared::{
    contracts::{CommandContext, Principal},
    ids::*,
};
use uuid::Uuid;

fn actor(n: u128) -> ActorId {
    ActorId::try_from(Uuid::from_u128(n)).unwrap()
}
fn unit(n: u128) -> ResourceId {
    ResourceId::try_from(Uuid::from_u128(n)).unwrap()
}
fn company(n: u128) -> CompanyScope {
    CompanyScope {
        tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
        company: CompanyId::try_from(Uuid::from_u128(n)).unwrap(),
    }
}
fn context(n: u128) -> CommandContext {
    CommandContext {
        command: CommandId::try_from(Uuid::from_u128(40)).unwrap(),
        correlation: CorrelationId::try_from(Uuid::from_u128(41)).unwrap(),
        actor: Principal::User(actor(n)),
        scope: ResourceScope::Company(company(10)),
        expected_version: None,
        occurred_at: "2026-01-01T00:00:00Z".parse().unwrap(),
    }
}
fn dossier() -> DossierFacts {
    DossierFacts {
        company: company(10),
        department: Some(unit(20)),
        owner: actor(2),
        reviewer: Some(actor(3)),
        department_confirmer: Some(actor(4)),
        approver: Some(actor(5)),
        reviewed_by: None,
        confirmed_by: None,
    }
}
fn grant(n: u128, capability: Capability, scope: GrantScope) -> Grant {
    Grant {
        company: company(10),
        actor: actor(n),
        capability,
        scope,
        active: true,
        valid_from: 10,
        valid_until: Some(20),
    }
}
fn check(
    n: u128,
    grants: &[Grant],
    depts: &[ResourceId],
    d: DossierFacts,
    op: Operation,
) -> Result<(), ScopedDenial> {
    Evaluation {
        context: &context(n),
        membership_active: true,
        active_departments: depts,
        grants,
        now: 15,
        dossier: d,
    }
    .decide(op)
}
#[test]
fn multiple_departments_no_cross_department_leak() {
    let grants = [
        grant(
            4,
            Capability::ReadDepartment,
            GrantScope::Department(unit(20)),
        ),
        grant(
            4,
            Capability::ReadDepartment,
            GrantScope::Department(unit(21)),
        ),
    ];
    assert_eq!(
        check(
            4,
            &grants,
            &[unit(20), unit(21)],
            dossier(),
            Operation::Read
        ),
        Ok(())
    );
    let mut d = dossier();
    d.department = Some(unit(21));
    assert_eq!(
        check(4, &grants, &[unit(20), unit(21)], d, Operation::Read),
        Ok(())
    );
    assert_eq!(
        check(4, &grants, &[unit(20)], d, Operation::Read),
        Err(ScopedDenial::MissingGrant)
    );
    d.department = Some(unit(22));
    assert_eq!(
        check(
            4,
            &grants,
            &[unit(20), unit(21), unit(22)],
            d,
            Operation::Read
        ),
        Err(ScopedDenial::MissingGrant)
    );
    d.department = None;
    assert_eq!(
        check(4, &grants, &[unit(20)], d, Operation::Read),
        Err(ScopedDenial::MissingGrant)
    );
}
#[test]
fn company_read_is_explicit_and_never_cross_company() {
    let mut g = grant(5, Capability::ReadCompany, GrantScope::Company);
    assert_eq!(check(5, &[g], &[], dossier(), Operation::Read), Ok(()));
    assert_eq!(
        check(4, &[g], &[], dossier(), Operation::Read),
        Err(ScopedDenial::MissingGrant)
    );
    g.company = company(11);
    assert_eq!(
        check(5, &[g], &[], dossier(), Operation::Read),
        Err(ScopedDenial::MissingGrant)
    );
    let mut d = dossier();
    d.company = company(11);
    assert_eq!(
        check(5, &[g], &[], d, Operation::Read),
        Err(ScopedDenial::Scope)
    );
}
#[test]
fn department_scope_cannot_promote_to_company() {
    let grants = [
        grant(4, Capability::ReadCompany, GrantScope::Department(unit(20))),
        grant(4, Capability::ReadDepartment, GrantScope::Company),
    ];
    assert_eq!(
        check(4, &grants, &[unit(20)], dossier(), Operation::Read),
        Err(ScopedDenial::MissingGrant)
    );
}
#[test]
fn expired_revoked_future_and_unassigned_membership_denied() {
    let mut g = grant(5, Capability::ReadCompany, GrantScope::Company);
    g.valid_until = Some(15);
    assert_eq!(
        check(5, &[g], &[], dossier(), Operation::Read),
        Err(ScopedDenial::MissingGrant)
    );
    g.valid_until = None;
    g.valid_from = 16;
    assert_eq!(
        check(5, &[g], &[], dossier(), Operation::Read),
        Err(ScopedDenial::MissingGrant)
    );
    g.valid_from = 10;
    g.active = false;
    assert_eq!(
        check(5, &[g], &[], dossier(), Operation::Read),
        Err(ScopedDenial::MissingGrant)
    );
    g.active = true;
    assert_eq!(
        Evaluation {
            context: &context(5),
            membership_active: false,
            active_departments: &[],
            grants: &[g],
            now: 15,
            dossier: dossier()
        }
        .decide(Operation::Read),
        Err(ScopedDenial::InactiveMembership)
    );
}
#[test]
fn multi_role_never_overrides_separation_of_duties() {
    let grants = [
        grant(2, Capability::WriteOwn, GrantScope::Company),
        grant(2, Capability::ReviewAssigned, GrantScope::Company),
        grant(2, Capability::ApproveAssigned, GrantScope::Company),
    ];
    let mut d = dossier();
    d.reviewer = Some(actor(2));
    d.approver = Some(actor(2));
    assert_eq!(check(2, &grants, &[], d, Operation::WriteDraft), Ok(()));
    assert_eq!(
        check(2, &grants, &[], d, Operation::Review),
        Err(ScopedDenial::SelfApproval)
    );
    assert_eq!(
        check(2, &grants, &[], d, Operation::Approve),
        Err(ScopedDenial::SelfApproval)
    );
    let g = grant(5, Capability::ApproveAssigned, GrantScope::Company);
    d = dossier();
    d.reviewed_by = Some(actor(5));
    assert_eq!(
        check(5, &[g], &[], d, Operation::Approve),
        Err(ScopedDenial::SelfApproval)
    );
    d.reviewed_by = None;
    d.confirmed_by = Some(actor(5));
    assert_eq!(
        check(5, &[g], &[], d, Operation::Approve),
        Err(ScopedDenial::SelfApproval)
    );
}
#[test]
fn explicit_assignment_and_independent_approval_required() {
    let g = grant(3, Capability::ReviewAssigned, GrantScope::Company);
    assert_eq!(check(3, &[g], &[], dossier(), Operation::Review), Ok(()));
    let mut d = dossier();
    d.reviewer = None;
    assert_eq!(
        check(3, &[g], &[], d, Operation::Review),
        Err(ScopedDenial::NotAssigned)
    );
    let g = grant(
        4,
        Capability::ConfirmDepartment,
        GrantScope::Department(unit(20)),
    );
    assert_eq!(
        check(
            4,
            &[g],
            &[unit(20)],
            dossier(),
            Operation::ConfirmDepartment
        ),
        Ok(())
    );
    let g = grant(5, Capability::ApproveAssigned, GrantScope::Company);
    assert_eq!(check(5, &[g], &[], dossier(), Operation::Approve), Ok(()));
    assert_eq!(
        check(5, &[], &[], dossier(), Operation::Approve),
        Err(ScopedDenial::MissingGrant)
    );
}
#[test]
fn worker_cannot_inherit_user_capabilities() {
    let mut ctx = context(5);
    ctx.actor = Principal::Worker(actor(5));
    let grants = [grant(5, Capability::ReadCompany, GrantScope::Company)];
    assert_eq!(
        Evaluation {
            context: &ctx,
            membership_active: true,
            active_departments: &[],
            grants: &grants,
            now: 15,
            dossier: dossier()
        }
        .decide(Operation::Read),
        Err(ScopedDenial::Worker)
    );
}

#[test]
fn custom_roles_and_many_assignments_resolve_without_scope_expansion() {
    let definitions = [
        RoleDefinition {
            company: company(10),
            id: unit(100),
            active: true,
            capabilities: vec![
                Capability::ReadDepartment,
                Capability::ConfirmDepartment,
                Capability::ReadCompany,
            ],
        },
        RoleDefinition {
            company: company(10),
            id: unit(101),
            active: true,
            capabilities: vec![Capability::ReadOwn, Capability::WriteOwn],
        },
    ];
    let assignment = |role, scope| RoleAssignment {
        company: company(10),
        actor: actor(4),
        role_id: unit(role),
        scope,
        active: true,
        valid_from: 10,
        valid_until: Some(20),
    };
    let assignments = [
        assignment(100, GrantScope::Department(unit(20))),
        assignment(100, GrantScope::Department(unit(21))),
        assignment(101, GrantScope::Company),
    ];
    let grants = resolve_grants(company(10), actor(4), &definitions, &assignments);
    assert_eq!(grants.len(), 6);
    assert!(
        grants
            .iter()
            .all(|g| g.capability != Capability::ReadCompany)
    );
    assert_eq!(
        check(4, &grants, &[unit(20)], dossier(), Operation::Read),
        Ok(())
    );
    let mut d = dossier();
    d.department = Some(unit(22));
    assert_eq!(
        check(4, &grants, &[unit(22)], d, Operation::Read),
        Err(ScopedDenial::MissingGrant)
    );
}

#[test]
fn inactive_or_ambiguous_role_definitions_never_grant_permissions() {
    let mut role = RoleDefinition {
        company: company(10),
        id: unit(100),
        active: true,
        capabilities: vec![Capability::ReadCompany],
    };
    let mut assignment = RoleAssignment {
        company: company(10),
        actor: actor(5),
        role_id: unit(100),
        scope: GrantScope::Company,
        active: true,
        valid_from: 10,
        valid_until: Some(20),
    };
    assert_eq!(
        resolve_grants(company(10), actor(5), &[role.clone()], &[assignment]).len(),
        1
    );
    assert!(
        resolve_grants(
            company(10),
            actor(5),
            &[role.clone(), role.clone()],
            &[assignment]
        )
        .is_empty()
    );
    role.active = false;
    assert!(resolve_grants(company(10), actor(5), &[role.clone()], &[assignment]).is_empty());
    role.active = true;
    assignment.active = false;
    assert!(resolve_grants(company(10), actor(5), &[role.clone()], &[assignment]).is_empty());
    assignment.active = true;
    assignment.company = company(11);
    assert!(resolve_grants(company(10), actor(5), &[role.clone()], &[assignment]).is_empty());
    assignment.company = company(10);
    assignment.valid_until = Some(10);
    assert!(resolve_grants(company(10), actor(5), &[role], &[assignment]).is_empty());
}


#[test]
fn resolved_department_assignments_enforce_membership_and_effective_time() {
    let definitions = [
        RoleDefinition {
            company: company(10),
            id: unit(100),
            active: true,
            capabilities: vec![Capability::ReadDepartment],
        },
        RoleDefinition {
            company: company(10),
            id: unit(101),
            active: true,
            capabilities: vec![Capability::ReadDepartment],
        },
    ];
    let assignment = |role_id, department_id, active, valid_from, valid_until| RoleAssignment {
        company: company(10),
        actor: actor(4),
        role_id: unit(role_id),
        scope: GrantScope::Department(unit(department_id)),
        active,
        valid_from,
        valid_until,
    };
    let assignments = [
        assignment(100, 20, true, 10, Some(20)),
        assignment(101, 21, true, 20, Some(30)),
        // A third department is active in HR but its permission assignment was revoked.
        assignment(101, 22, false, 10, None),
    ];
    let grants = resolve_grants(company(10), actor(4), &definitions, &assignments);

    let decide = |department, now, active_departments: &[ResourceId]| {
        let mut facts = dossier();
        facts.department = Some(unit(department));
        Evaluation {
            context: &context(4),
            membership_active: true,
            active_departments,
            grants: &grants,
            now,
            dossier: facts,
        }
        .decide(Operation::Read)
    };

    assert_eq!(decide(20, 10, &[unit(20), unit(21), unit(22)]), Ok(()));
    assert_eq!(
        decide(20, 19, &[unit(20), unit(21), unit(22)]),
        Ok(())
    );
    // A future-dated grant cannot be used before its inclusive start.
    assert_eq!(
        decide(21, 19, &[unit(20), unit(21)]),
        Err(ScopedDenial::MissingGrant)
    );
    // The first assignment expires exactly when the second becomes effective.
    assert_eq!(
        decide(20, 20, &[unit(20), unit(21)]),
        Err(ScopedDenial::MissingGrant)
    );
    assert_eq!(decide(21, 20, &[unit(20), unit(21)]), Ok(()));
    assert_eq!(
        decide(21, 30, &[unit(20), unit(21)]),
        Err(ScopedDenial::MissingGrant)
    );
    // Membership in a department cannot substitute for a revoked role assignment.
    assert_eq!(
        decide(22, 15, &[unit(20), unit(21), unit(22)]),
        Err(ScopedDenial::MissingGrant)
    );
    // A grant is ineffective if the actor's department membership is no longer active.
    assert_eq!(
        decide(21, 20, &[unit(20)]),
        Err(ScopedDenial::MissingGrant)
    );
}
