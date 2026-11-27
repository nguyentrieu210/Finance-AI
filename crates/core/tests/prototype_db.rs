use finance_core::prototype::*;
use finance_shared::{
    contracts::{CommandContext, Principal},
    http_contract::IdempotencyKey,
    ids::*,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn ctx(command: u128, tenant: u128, company: u128, actor: u128) -> CommandContext {
    CommandContext {
        command: CommandId::try_from(id(command)).unwrap(),
        correlation: CorrelationId::try_from(id(command + 1000)).unwrap(),
        actor: Principal::User(ActorId::try_from(id(actor)).unwrap()),
        scope: ResourceScope::Company(CompanyScope {
            tenant: TenantId::try_from(id(tenant)).unwrap(),
            company: CompanyId::try_from(id(company)).unwrap(),
        }),
        expected_version: None,
        occurred_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
    }
}
#[tokio::test]
#[ignore = "requires isolated synthetic PostgreSQL; run scripts/test_prototype_foundation.sh"]
async fn real_database_transaction_and_scope() {
    let url = std::env::var("FINANCE_PROTOTYPE_TEST_DATABASE_URL").expect("synthetic database URL");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE finance_runtime")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let a = ctx(100, 1, 10, 1000);
    let key = IdempotencyKey::parse("first").unwrap();
    let first = CreateUnit {
        id: id(101),
        parent: None,
        code: "DEPT-A",
        name: "Synthetic department",
        kind: UnitKind::Department,
    };
    assert_eq!(
        create_unit(&pool, &a, &key, &first).await.unwrap(),
        Receipt::Created(id(101))
    );
    assert_eq!(
        create_unit(&pool, &ctx(102, 1, 10, 1000), &key, &first)
            .await
            .unwrap(),
        Receipt::Replayed(id(101))
    );
    let changed = CreateUnit {
        name: "changed",
        ..first
    };
    assert!(matches!(
        create_unit(&pool, &a, &key, &changed).await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        create_unit(&pool, &ctx(103, 1, 10, 1001), &key, &first).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        create_unit(&pool, &ctx(103, 1, 10, 1002), &key, &first).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        create_unit(&pool, &ctx(103, 1, 11, 1000), &key, &first).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        create_unit(&pool, &ctx(103, 2, 20, 1000), &key, &first).await,
        Err(Error::Forbidden)
    ));
    // Separate concurrent transactions using same key; only one mutation is allowed.
    let key2 = IdempotencyKey::parse("concurrent").unwrap();
    let second = CreateUnit {
        id: id(201),
        parent: Some(id(101)),
        code: "COST-A",
        name: "Synthetic cost center",
        kind: UnitKind::CostCenter,
    };
    let b = ctx(200, 1, 10, 1000);
    let c = ctx(202, 1, 10, 1000);
    let (x, y) = tokio::join!(
        create_unit(&pool, &b, &key2, &second),
        create_unit(&pool, &c, &key2, &second)
    );
    let (x, y) = (x.unwrap(), y.unwrap());
    assert!(matches!(
        (x, y),
        (Receipt::Created(_), Receipt::Replayed(_)) | (Receipt::Replayed(_), Receipt::Created(_))
    ));
    // Duplicate command ID forces audit insert failure after unit insert; all side effects roll back.
    let failed = CreateUnit {
        id: id(301),
        parent: None,
        code: "ROLLBACK",
        name: "rollback probe",
        kind: UnitKind::Department,
    };
    assert!(matches!(
        create_unit(
            &pool,
            &a,
            &IdempotencyKey::parse("rollback").unwrap(),
            &failed
        )
        .await,
        Err(Error::Database(_))
    ));
    let mut tx = scoped_transaction(&pool, &a).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM finance.organization_unit")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 2);
    let failed_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM finance.organization_unit WHERE code='ROLLBACK'")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    assert_eq!(failed_count, 0);
    tx.commit().await.unwrap();
    let mut tx = scoped_transaction(&pool, &ctx(300, 1, 11, 1000))
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM finance.organization_unit")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 1);
    tx.commit().await.unwrap();
    let mut tx = scoped_transaction(&pool, &ctx(301, 2, 20, 2000))
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM finance.organization_unit")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(count, 1);
    tx.commit().await.unwrap();
    // Pool reuse outside a scoped transaction must not carry previous tenant/company.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM finance.organization_unit")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    pool.close().await;
}
