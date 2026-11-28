use super::{Lease, RunFailure, preflight_error};
use finance_documents::{
    Binding,
    s3::{Object, Phase, S3},
};
use finance_shared::{
    contracts::Version,
    ids::{ResourceId, RevisionId},
    upload_contract::{DeclaredMime, Sha256Digest},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;
#[derive(sqlx::FromRow)]
struct Source {
    material: Value,
    document_id: Uuid,
    revision_id: Uuid,
    generation: i64,
    mime: String,
    sha256: String,
    byte_count: i64,
    object_version: Option<String>,
    content: Option<Vec<u8>>,
    format: String,
}
pub async fn load(pool: &PgPool, lease: Lease) -> Result<Vec<u8>, RunFailure> {
    let scope = lease.scope();
    let (t, c): (Uuid, Uuid) = (scope.tenant.into(), scope.company.into());
    let mut tx = pool.begin().await.map_err(preflight_error)?;
    sqlx::query(
        "SELECT set_config('finance.tenant_id',$1,true),set_config('finance.company_id',$2,true)",
    )
    .bind(t.to_string())
    .bind(c.to_string())
    .execute(&mut *tx)
    .await
    .map_err(preflight_error)?;
    let row = sqlx::query_as::<_, Source>(
        "SELECT * FROM finance.runtime_export_material($1,$2,$3,$4,$5)",
    )
    .bind(t)
    .bind(c)
    .bind(lease.job_id)
    .bind(lease.fence)
    .bind(lease.worker_id())
    .fetch_optional(&mut *tx)
    .await
    .map_err(preflight_error)?
    .ok_or(RunFailure::Permanent)?;
    tx.commit().await.map_err(preflight_error)?;
    if !(1..=26214400).contains(&row.byte_count) {
        return Err(RunFailure::Permanent);
    }
    let bytes = if let Some(version) = row.object_version {
        let storage = S3::from_env()
            .map_err(|_| RunFailure::Permanent)?
            .ok_or(RunFailure::Permanent)?;
        let object = Object {
            binding: Binding {
                scope,
                document: ResourceId::try_from(row.document_id)
                    .map_err(|_| RunFailure::Permanent)?,
                revision: RevisionId::try_from(row.revision_id)
                    .map_err(|_| RunFailure::Permanent)?,
            },
            generation: Version::try_from(row.generation).map_err(|_| RunFailure::Permanent)?,
            mime: DeclaredMime::parse(&row.mime).map_err(|_| RunFailure::Permanent)?,
            size: row.byte_count as u64,
            digest: Sha256Digest::parse(&row.sha256).map_err(|_| RunFailure::Permanent)?,
        };
        storage
            .read_verified(
                &object,
                Phase::Quarantine,
                &version,
                chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now()),
            )
            .await
            .map_err(|_| RunFailure::Transient)?
    } else {
        row.content.ok_or(RunFailure::Permanent)?
    };
    if bytes.len() != row.byte_count as usize
        || format!("{:x}", Sha256::digest(&bytes)) != row.sha256
    {
        return Err(RunFailure::Permanent);
    }
    let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    serde_json::to_vec(&json!({"material":row.material,"format":row.format,"template_hex":hex,"template_sha256":row.sha256})).map_err(|_|RunFailure::Permanent)
}
