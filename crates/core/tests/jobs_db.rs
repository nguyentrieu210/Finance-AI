use finance_core::jobs::*;
use finance_shared::ids::{CompanyId, CompanyScope, TenantId};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use uuid::Uuid;
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn scope(t: u128, c: u128) -> CompanyScope {
    CompanyScope {
        tenant: TenantId::try_from(id(t)).unwrap(),
        company: CompanyId::try_from(id(c)).unwrap(),
    }
}
#[tokio::test]
#[ignore = "requires disposable PostgreSQL; scripts/test_prototype_foundation.sh"]
async fn durable_jobs_real_database() {
    let url =
        std::env::var("FINANCE_JOB_TEST_DATABASE_URL").expect("synthetic worker database URL");
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE finance_job_worker")
                    .execute(c)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let s = scope(1, 10);
    assert!(matches!(
        dispatch(&pool, scope(1, 11), 10).await,
        Err(JobError::Forbidden)
    ));
    assert!(matches!(
        dispatch(&pool, scope(2, 20), 10).await,
        Err(JobError::Forbidden)
    ));
    assert!(matches!(
        dispatch(&pool, s, 0).await,
        Err(JobError::InvalidInput)
    ));
    assert!(matches!(
        dispatch_event(&pool, s, "bad-event", 10).await,
        Err(JobError::InvalidInput)
    ));
    assert_eq!(
        dispatch_event(&pool, s, "EXPORT_REQUESTED", 100)
            .await
            .unwrap(),
        0
    );
    assert!(
        claim_event(&pool, s, id(4999), 30, "EXPORT_REQUESTED")
            .await
            .unwrap()
            .is_none()
    );
    let (a, b) = tokio::join!(dispatch(&pool, s, 100), dispatch(&pool, s, 100));
    assert_eq!(a.unwrap() + b.unwrap(), 2);
    assert_eq!(dispatch(&pool, s, 100).await.unwrap(), 0);
    let (a, b) = tokio::join!(claim(&pool, s, id(5001), 1), claim(&pool, s, id(5002), 30));
    let first = a.unwrap().unwrap();
    let second = b.unwrap().unwrap();
    assert_ne!(first.job_id, second.job_id);
    assert_eq!(first.fence, 1);
    assert_eq!(second.fence, 1);
    assert!(claim(&pool, s, id(5003), 30).await.unwrap().is_none());
    heartbeat(&pool, second, 30).await.unwrap();
    complete(&pool, second).await.unwrap();
    assert!(matches!(
        complete(&pool, second).await,
        Err(JobError::StaleLease)
    ));
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(matches!(
        heartbeat(&pool, first, 30).await,
        Err(JobError::StaleLease)
    ));
    let recovered = claim(&pool, s, id(5003), 30).await.unwrap().unwrap();
    assert_eq!(recovered.job_id, first.job_id);
    assert_eq!(recovered.fence, 2);
    assert!(matches!(
        complete(&pool, first).await,
        Err(JobError::StaleLease)
    ));
    assert_eq!(
        fail(&pool, recovered, Failure::Transient).await.unwrap(),
        FailureResult::Retry
    );
    assert!(claim(&pool, s, id(5004), 30).await.unwrap().is_none());
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let third = claim(&pool, s, id(5004), 30).await.unwrap().unwrap();
    assert_eq!(third.fence, 3);
    assert_eq!(
        fail(&pool, third, Failure::Transient).await.unwrap(),
        FailureResult::Dead
    );
    assert!(claim(&pool, s, id(5005), 30).await.unwrap().is_none());
    let unscoped: i64 = sqlx::query_scalar("SELECT count(*) FROM finance.job")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(unscoped, 0);
    assert!(
        sqlx::query("DELETE FROM finance.inbox")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE finance.outbox SET event_type='ORG_UNIT_CREATED'")
            .execute(&pool)
            .await
            .is_err()
    );
    pool.close().await;
}
