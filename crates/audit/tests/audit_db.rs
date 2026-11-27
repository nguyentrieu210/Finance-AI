use finance_audit::*;
use finance_core::prototype::{CreateUnit, Error, UnitKind, create_unit, scoped_transaction};
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
fn ctx(n: u128, t: u128, c: u128, a: u128) -> CommandContext {
    CommandContext {
        command: CommandId::try_from(id(n)).unwrap(),
        correlation: CorrelationId::try_from(id(n + 10000)).unwrap(),
        actor: Principal::User(ActorId::try_from(id(a)).unwrap()),
        scope: ResourceScope::Company(CompanyScope {
            tenant: TenantId::try_from(id(t)).unwrap(),
            company: CompanyId::try_from(id(c)).unwrap(),
        }),
        expected_version: None,
        occurred_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
    }
}
#[tokio::test]
#[ignore = "disposable synthetic PostgreSQL only"]
async fn scoped_audit_real_database() {
    let url = std::env::var("FINANCE_PROTOTYPE_TEST_DATABASE_URL").unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE finance_runtime").execute(c).await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    // Seed actual events outside the auditor company after the earlier integration assertions.
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("FINANCE_AUDIT_TEST_ADMIN_URL").unwrap())
        .await
        .unwrap();
    for (t, c, a, n) in [(1, 11, 1000, 8200), (2, 20, 2000, 8201)] {
        sqlx::query("INSERT INTO finance.membership(tenant_id,company_id,actor_id,role) VALUES($1,$2,$3,'ORG_ADMIN')")
            .bind(id(t)).bind(id(c)).bind(id(a)).execute(&admin).await.unwrap();
        create_unit(
            &pool,
            &ctx(n, t, c, a),
            &IdempotencyKey::parse("audit-cross-scope").unwrap(),
            &CreateUnit {
                id: id(n + 100),
                parent: None,
                code: "AUDIT-PROBE",
                name: "Synthetic audit probe",
                kind: UnitKind::Department,
            },
        )
        .await
        .unwrap();
    }
    let mut tx = scoped_transaction(&pool, &ctx(8300, 1, 10, 1003))
        .await
        .unwrap();
    let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM finance.audit_event")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(visible, 2);
    tx.commit().await.unwrap();
    assert!(matches!(
        read_page(&pool, &ctx(8001, 1, 10, 1001), 0, 100).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        read_page(&pool, &ctx(8002, 1, 11, 1003), 0, 100).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        read_page(&pool, &ctx(8003, 2, 20, 1003), 0, 100).await,
        Err(Error::Forbidden)
    ));
    let ca = ctx(8100, 1, 10, 1003);
    let cb = ctx(8101, 1, 10, 1003);
    let (a, b) = tokio::join!(read_page(&pool, &ca, 0, 1), read_page(&pool, &cb, 0, 100));
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 2);
    assert!(b.iter().all(|e| e.company == id(10) && e.tenant == id(1)));
    assert_eq!(
        read_page(&pool, &ctx(8102, 1, 10, 1003), a[0].sequence, 100)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        sqlx::query("DELETE FROM finance.audit_event")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE finance.audit_event SET action='AUDIT_READ'")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT * FROM finance.audit_chain_head")
            .fetch_all(&pool)
            .await
            .is_err()
    );
    let unscoped: i64 = sqlx::query_scalar("SELECT count(*) FROM finance.audit_event")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(unscoped, 0);
    pool.close().await;
    // privileged independent verifier is test-only and sees full chain, never exposed to runtime.
    let sql = format!(
        "SELECT {EVENT_FIELDS} FROM finance.audit_event WHERE tenant_id=$1 ORDER BY sequence"
    );
    let rows = sqlx::query(&sql)
        .bind(id(1))
        .fetch_all(&admin)
        .await
        .unwrap();
    let events = rows
        .iter()
        .map(decode_event)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let (seq, digest): (i64, Vec<u8>) =
        sqlx::query_as("SELECT sequence,digest FROM finance.audit_chain_head WHERE tenant_id=$1")
            .bind(id(1))
            .fetch_one(&admin)
            .await
            .unwrap();
    let head = Head {
        sequence: seq,
        digest: digest.try_into().unwrap(),
    };
    assert_eq!(head.sequence, 6);
    let other_rows = sqlx::query(&sql)
        .bind(id(2))
        .fetch_all(&admin)
        .await
        .unwrap();
    let other_events = other_rows
        .iter()
        .map(decode_event)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let (other_seq, other_digest): (i64, Vec<u8>) =
        sqlx::query_as("SELECT sequence,digest FROM finance.audit_chain_head WHERE tenant_id=$1")
            .bind(id(2))
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(other_seq, 1);
    assert_eq!(
        verify_chain(
            id(2),
            other_events,
            Head {
                sequence: other_seq,
                digest: other_digest.try_into().unwrap()
            }
        ),
        Ok(())
    );
    assert_eq!(verify_chain(id(1), events.clone(), head), Ok(()));
    // Real row tampering in a rolled-back test transaction.
    let mut tx = admin.begin().await.unwrap();
    sqlx::query("UPDATE finance.audit_event SET resource_id=$1 WHERE tenant_id=$2 AND sequence=1")
        .bind(id(9999))
        .bind(id(1))
        .execute(&mut *tx)
        .await
        .unwrap();
    let rows = sqlx::query(&sql)
        .bind(id(1))
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    let changed = rows
        .iter()
        .map(decode_event)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        verify_chain(id(1), changed, head),
        Err(IntegrityError::DigestMismatch)
    );
    tx.rollback().await.unwrap();
    assert_eq!(
        verify_chain(id(1), events.into_iter().take(5), head),
        Err(IntegrityError::HeadMismatch)
    );
    admin.close().await;
}
