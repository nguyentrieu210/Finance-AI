use finance_core::prototype::*;
use finance_permissions::{GrantError, Role, require_role};
use finance_shared::{
    contracts::{CommandContext, Principal},
    http_contract::IdempotencyKey,
    ids::*,
};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use uuid::Uuid;
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn ctx(n: u128, a: u128) -> CommandContext {
    CommandContext {
        command: CommandId::try_from(id(n)).unwrap(),
        correlation: CorrelationId::try_from(id(n + 10000)).unwrap(),
        actor: Principal::User(ActorId::try_from(id(a)).unwrap()),
        scope: ResourceScope::Company(CompanyScope {
            tenant: TenantId::try_from(id(1)).unwrap(),
            company: CompanyId::try_from(id(10)).unwrap(),
        }),
        expected_version: None,
        occurred_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
    }
}
#[tokio::test]
#[ignore = "requires disposable synthetic PostgreSQL"]
async fn permission_revocation_real_database() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
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
        .max_connections(2)
        .connect(&std::env::var("FINANCE_AUDIT_TEST_ADMIN_URL").unwrap())
        .await
        .unwrap();
    let c = ctx(9001, 1000);
    let mut tx = scoped_transaction(&pool, &c).await.unwrap();
    assert!(matches!(
        require_role(&mut tx, &ctx(9002, 1003), Role::Auditor).await,
        Err(GrantError::Forbidden)
    ));
    require_role(&mut tx, &c, Role::OrgAdmin).await.unwrap();
    let revoke = "UPDATE finance.membership SET active=false WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND role='ORG_ADMIN'";
    // If command obtains row lock first, revoke must wait for transaction completion.
    let pending = sqlx::query(revoke)
        .bind(id(1))
        .bind(id(10))
        .bind(id(1000))
        .execute(&admin);
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(250), &mut pending)
            .await
            .is_err()
    );
    tx.commit().await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), &mut pending)
            .await
            .unwrap()
            .unwrap()
            .rows_affected(),
        1
    );
    let input = CreateUnit {
        id: id(9003),
        parent: None,
        code: "REVOKE-PROBE",
        name: "Synthetic denied command",
        kind: UnitKind::Department,
    };
    let key = IdempotencyKey::parse("revoke-probe").unwrap();
    assert!(matches!(
        create_unit(&pool, &c, &key, &input).await,
        Err(Error::Forbidden)
    ));
    // Restore grant in disposable fixture only, then let revocation own the row first.
    sqlx::query("UPDATE finance.membership SET active=true WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND role='ORG_ADMIN'").bind(id(1)).bind(id(10)).bind(id(1000)).execute(&admin).await.unwrap();
    let mut revoker = admin.begin().await.unwrap();
    sqlx::query(revoke)
        .bind(id(1))
        .bind(id(10))
        .bind(id(1000))
        .execute(&mut *revoker)
        .await
        .unwrap();
    let command = create_unit(&pool, &c, &key, &input);
    tokio::pin!(command);
    assert!(
        tokio::time::timeout(Duration::from_millis(250), &mut command)
            .await
            .is_err()
    );
    revoker.commit().await.unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), &mut command)
            .await
            .unwrap(),
        Err(Error::Forbidden)
    ));
    // Even an existing idempotent receipt must not replay after permission is revoked.
    let first = CreateUnit {
        id: id(101),
        parent: None,
        code: "DEPT-A",
        name: "Synthetic department",
        kind: UnitKind::Department,
    };
    assert!(matches!(
        create_unit(&pool, &c, &IdempotencyKey::parse("first").unwrap(), &first).await,
        Err(Error::Forbidden)
    ));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM finance.organization_unit WHERE id=$1")
            .bind(id(9003))
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(count, 0);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM finance.audit_event WHERE command_id=$1")
            .bind(id(9001))
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(count, 0);
    assert!(
        sqlx::query("SET ROLE finance_permission_guard")
            .execute(&pool)
            .await
            .is_err()
    );
    pool.close().await;
    admin.close().await;
}
