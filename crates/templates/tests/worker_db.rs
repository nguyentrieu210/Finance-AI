use finance_core::jobs;
use finance_shared::{
    ids::{CompanyId, CompanyScope, TenantId},
    upload_contract::Sha256Digest,
};
use finance_templates::{
    Artifact,
    export::{WorkerError, complete},
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}
fn scope() -> CompanyScope {
    CompanyScope {
        tenant: TenantId::try_from(id(1)).unwrap(),
        company: CompanyId::try_from(id(10)).unwrap(),
    }
}

#[tokio::test]
#[ignore = "requires disposable PostgreSQL after API export request"]
async fn fenced_export_worker_real_database() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE finance_job_worker")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&std::env::var("FINANCE_JOB_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    jobs::dispatch(&pool, scope(), 100).await.unwrap();
    let digest = Sha256Digest::parse(&"a".repeat(64)).unwrap();
    let artifacts = [
        Artifact {
            name: "request.docx",
            expected: digest,
            observed: Some(digest),
            bytes: 100,
        },
        Artifact {
            name: "checklist.xlsx",
            expected: digest,
            observed: Some(digest),
            bytes: 120,
        },
        Artifact {
            name: "request.pdf",
            expected: digest,
            observed: Some(digest),
            bytes: 140,
        },
    ];
    let mut exported = false;
    for worker in 6000_u128..6100 {
        let Some(lease) = jobs::claim(&pool, scope(), id(worker), 30).await.unwrap() else {
            break;
        };
        match complete(&pool, lease, &artifacts).await {
            Ok(version) => {
                assert_eq!(version, 2);
                exported = true;
                break;
            }
            Err(WorkerError::NotExportJob) => {
                jobs::complete(&pool, lease).await.unwrap();
            }
            Err(WorkerError::StaleLease) => panic!("fresh export lease rejected"),
            Err(WorkerError::InvalidManifest) => panic!("verified manifest rejected"),
            Err(WorkerError::Forbidden) => panic!("scoped worker rejected"),
            Err(WorkerError::Database(error)) => panic!("export database error: {error}"),
        }
    }
    assert!(exported, "queued export job was not reached");
    pool.close().await;
}
