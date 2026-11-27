use finance_core::prototype::{CreateUnit, Error, UnitKind, create_unit};
use finance_organization::{UpdateUnit, list_units, update_unit};
use finance_shared::{
    contracts::{CommandContext, Principal, Version},
    http_contract::IdempotencyKey,
    ids::*,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn ctx(n: u128, c: u128, a: u128, v: Option<i64>) -> CommandContext {
    CommandContext {
        command: CommandId::try_from(id(n)).unwrap(),
        correlation: CorrelationId::try_from(id(n + 10000)).unwrap(),
        actor: Principal::User(ActorId::try_from(id(a)).unwrap()),
        scope: ResourceScope::Company(CompanyScope {
            tenant: TenantId::try_from(id(1)).unwrap(),
            company: CompanyId::try_from(id(c)).unwrap(),
        }),
        expected_version: v.map(|v| Version::try_from(v).unwrap()),
        occurred_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
    }
}
fn key(s: &str) -> IdempotencyKey {
    IdempotencyKey::parse(s).unwrap()
}
#[tokio::test]
#[ignore = "requires disposable synthetic PostgreSQL"]
async fn organization_real_database() {
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE finance_runtime").execute(c).await?;
                Ok(())
            })
        })
        .connect(&std::env::var("FINANCE_PROTOTYPE_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("FINANCE_AUDIT_TEST_ADMIN_URL").unwrap())
        .await
        .unwrap();
    // Prior revocation test disabled this grant. Restore only the disposable fixture.
    sqlx::query("UPDATE finance.membership SET active=true WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND role='ORG_ADMIN'").bind(id(1)).bind(id(10)).bind(id(1000)).execute(&admin).await.unwrap();
    for (n, code) in [(9101, "TREE-A"), (9102, "TREE-B")] {
        create_unit(
            &pool,
            &ctx(n + 100, 10, 1000, None),
            &key(code),
            &CreateUnit {
                id: id(n),
                parent: None,
                code,
                name: "Synthetic tree node",
                kind: UnitKind::Department,
            },
        )
        .await
        .unwrap();
    }
    let a = UpdateUnit {
        id: id(9101),
        parent: Some(id(9102)),
        name: "A under B",
        active: true,
    };
    let b = UpdateUnit {
        id: id(9102),
        parent: Some(id(9101)),
        name: "B under A",
        active: true,
    };
    let ca = ctx(9301, 10, 1000, Some(0));
    let cb = ctx(9302, 10, 1000, Some(0));
    let ka = key("move-a");
    let kb = key("move-b");
    let (ra, rb) = tokio::join!(
        update_unit(&pool, &ca, &ka, &a),
        update_unit(&pool, &cb, &kb, &b)
    );
    assert!(matches!(
        (&ra, &rb),
        (Ok(_), Err(Error::Conflict)) | (Err(Error::Conflict), Ok(_))
    ));
    let (child, parent, command, k, input) = if ra.is_ok() {
        (id(9101), id(9102), ca, ka, &a)
    } else {
        (id(9102), id(9101), cb, kb, &b)
    };
    let replay = update_unit(&pool, &command, &k, input).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(i64::from(replay.version), 1);
    assert!(matches!(
        update_unit(&pool, &command, &key("stale"), input).await,
        Err(Error::Conflict)
    ));
    let disable_parent = UpdateUnit {
        id: parent,
        parent: None,
        name: "Disabled parent",
        active: false,
    };
    assert!(matches!(
        update_unit(
            &pool,
            &ctx(9401, 10, 1000, Some(0)),
            &key("disable-parent"),
            &disable_parent
        )
        .await,
        Err(Error::Conflict)
    ));
    let disable_child = UpdateUnit {
        id: child,
        parent: Some(parent),
        name: "Disabled child",
        active: false,
    };
    let r = update_unit(
        &pool,
        &ctx(9402, 10, 1000, Some(1)),
        &key("disable-child"),
        &disable_child,
    )
    .await
    .unwrap();
    assert_eq!(i64::from(r.version), 2);
    let r = update_unit(
        &pool,
        &ctx(9403, 10, 1000, Some(0)),
        &key("disable-parent-ok"),
        &disable_parent,
    )
    .await
    .unwrap();
    assert_eq!(i64::from(r.version), 1);
    let reactivate = UpdateUnit {
        active: true,
        ..disable_child
    };
    assert!(matches!(
        update_unit(
            &pool,
            &ctx(9404, 10, 1000, Some(2)),
            &key("reactivate"),
            &reactivate
        )
        .await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        create_unit(
            &pool,
            &ctx(9405, 10, 1000, None),
            &key("new-under-inactive"),
            &CreateUnit {
                id: id(9406),
                parent: Some(parent),
                code: "INACTIVE-PROBE",
                name: "Must reject",
                kind: UnitKind::CostCenter
            }
        )
        .await,
        Err(Error::Database(_))
    ));
    let foreign_parent = UpdateUnit {
        id: child,
        parent: Some(id(1025)),
        name: "cross company",
        active: false,
    };
    assert!(matches!(
        update_unit(
            &pool,
            &ctx(9407, 10, 1000, Some(2)),
            &key("foreign-parent"),
            &foreign_parent
        )
        .await,
        Err(Error::Conflict)
    ));
    let before = update_unit(&pool, &command, &k, input).await.unwrap();
    assert!(before.replayed);
    assert_eq!(i64::from(before.version), 1);
    assert!(matches!(
        list_units(&pool, &ctx(9408, 10, 1001, None), Uuid::nil(), 100).await,
        Err(Error::Forbidden)
    ));
    let rows = list_units(&pool, &ctx(9409, 10, 1000, None), Uuid::nil(), 100)
        .await
        .unwrap();
    assert!(
        rows.iter()
            .any(|r| r.id == child && !r.active && r.version == 2)
    );
    assert!(
        rows.iter()
            .any(|r| r.id == parent && !r.active && r.version == 1)
    );
    assert!(!rows.iter().any(|r| r.id == id(1025)));
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM finance.audit_event WHERE action='ORG_UNIT_UPDATED'",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    assert_eq!(count, 3);
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM finance.outbox WHERE event_type='ORG_UNIT_UPDATED'",
    )
    .fetch_one(&admin)
    .await
    .unwrap();
    assert_eq!(count, 3);
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM finance.idempotency_record WHERE operation_id='prototype.updateOrganizationUnit'").fetch_one(&admin).await.unwrap();
    assert_eq!(count, 3);
    pool.close().await;
    admin.close().await;
}
