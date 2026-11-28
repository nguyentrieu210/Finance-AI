//! E20 export queue and worker receipt boundary.
use crate::{Artifact, verified_manifest};
use finance_core::{
    jobs::Lease,
    prototype::{Error, recheck_authenticated, scoped_transaction},
};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
    ids::{CompanyScope, ResourceScope},
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct Job {
    pub id: Uuid,
    pub state: &'static str,
    pub attempt: i64,
    pub progress: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip)]
    pub dossier_version: i64,
}
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, sqlx::FromRow)]
pub struct Package {
    pub id: Uuid,
    pub snapshot_id: Uuid,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_hash: Option<String>,
    pub created_at: String,
}

#[derive(Debug)]
pub enum WorkerError {
    InvalidManifest,
    NotExportJob,
    Forbidden,
    StaleLease,
    Database(sqlx::Error),
}
impl From<sqlx::Error> for WorkerError {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value)
    }
}

fn binding(proof: &AuthenticatedCommand) -> Result<(Uuid, Uuid, Uuid), Error> {
    let ResourceScope::Company(CompanyScope { tenant, company }) = proof.context().scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(actor) = proof.context().actor else {
        return Err(Error::Forbidden);
    };
    Ok((tenant.into(), company.into(), actor.into()))
}
fn hash_request(dossier: Uuid, snapshot: Uuid, template: Uuid, version: i64) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"finance.export.request/v1");
    hash.update(dossier.as_bytes());
    hash.update(snapshot.as_bytes());
    hash.update(template.as_bytes());
    hash.update(version.to_be_bytes());
    hash.finalize().to_vec()
}
pub fn manifest_digest(artifacts: &[Artifact<'_>]) -> Option<([u8; 32], i64)> {
    if !verified_manifest(artifacts) {
        return None;
    }
    let mut ordered: Vec<_> = artifacts.iter().collect();
    ordered.sort_by_key(|artifact| artifact.name);
    let mut total = 0_u64;
    let mut hash = Sha256::new();
    hash.update(b"finance.package.manifest/v1");
    for artifact in ordered {
        total = total.checked_add(artifact.bytes)?;
        hash.update((artifact.name.len() as u64).to_be_bytes());
        hash.update(artifact.name.as_bytes());
        hash.update(artifact.expected.as_bytes());
        hash.update(artifact.bytes.to_be_bytes());
    }
    let total = i64::try_from(total).ok()?;
    Some((hash.finalize().into(), total))
}

pub async fn request(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    dossier: Uuid,
    snapshot: Uuid,
    template_ids: &[Uuid],
    package: Uuid,
) -> Result<Job, Error> {
    let Some(expected) = proof.context().expected_version else {
        return Err(Error::InvalidInput);
    };
    let version = i64::from(expected);
    if dossier.is_nil()
        || snapshot.is_nil()
        || package.is_nil()
        || template_ids.len() != 1
        || template_ids[0].is_nil()
    {
        return Err(Error::InvalidInput);
    }
    let template = template_ids[0];
    let (tenant, company, actor) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|error| match error {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(error) => Error::Database(error),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "{tenant}/{company}/{actor}/export.request/{}",
            key.as_str()
        ))
        .execute(&mut *tx)
        .await?;
    let digest = hash_request(dossier, snapshot, template, version);
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='export.request' AND key=$4",
    )
    .bind(tenant)
    .bind(company)
    .bind(actor)
    .bind(key.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((old, job)) = prior {
        if old != digest {
            return Err(Error::Conflict);
        }
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM finance.dossier WHERE tenant_id=$1 AND company_id=$2 AND owner_id=$3 AND id=$4)",
        )
        .bind(tenant)
        .bind(company)
        .bind(actor)
        .bind(dossier)
        .fetch_one(&mut *tx)
        .await?;
        if !owned {
            return Err(Error::Forbidden);
        }
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(Job {
            id: job,
            state: "QUEUED",
            attempt: 0,
            progress: 0,
            error_code: None,
            dossier_version: version,
        });
    }
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM finance.dossier_snapshot s JOIN finance.dossier d ON (d.tenant_id,d.company_id,d.id)=(s.tenant_id,s.company_id,s.dossier_id) JOIN finance.template_version tv ON (tv.tenant_id,tv.company_id,tv.id)=(s.tenant_id,s.company_id,s.template_version_id) WHERE s.tenant_id=$1 AND s.company_id=$2 AND s.id=$3 AND s.dossier_id=$4 AND d.owner_id=$5 AND d.status='READY' AND d.version=$6 AND tv.id=$7 AND tv.state='PUBLISHED')",
    )
    .bind(tenant)
    .bind(company)
    .bind(snapshot)
    .bind(dossier)
    .bind(actor)
    .bind(version)
    .bind(template)
    .fetch_one(&mut *tx)
    .await?;
    if !valid {
        return Err(Error::Conflict);
    }
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query(
        "INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'EXPORT_REQUESTED')",
    )
    .bind(tenant)
    .bind(company)
    .bind(command)
    .bind(actor)
    .bind(correlation)
    .bind(dossier)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'EXPORT_REQUESTED')",
    )
    .bind(tenant)
    .bind(company)
    .bind(command)
    .bind(dossier)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO finance.package(tenant_id,company_id,id,snapshot_id,template_id,state,job_id,requested_by,dossier_version) VALUES($1,$2,$3,$4,$5,'QUEUED',$6,$7,$8)",
    )
    .bind(tenant)
    .bind(company)
    .bind(package)
    .bind(snapshot)
    .bind(template)
    .bind(command)
    .bind(actor)
    .bind(version)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'export.request',$4,$5,$6)",
    )
    .bind(tenant)
    .bind(company)
    .bind(actor)
    .bind(key.as_str())
    .bind(digest)
    .bind(command)
    .execute(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(Job {
        id: command,
        state: "QUEUED",
        attempt: 0,
        progress: 0,
        error_code: None,
        dossier_version: version,
    })
}

pub async fn list(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    after: Uuid,
    limit: i64,
) -> Result<Option<Vec<Package>>, Error> {
    if dossier.is_nil() || !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, actor) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|error| match error {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(error) => Error::Database(error),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let owned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM finance.dossier WHERE tenant_id=$1 AND company_id=$2 AND owner_id=$3 AND id=$4)",
    )
    .bind(tenant)
    .bind(company)
    .bind(actor)
    .bind(dossier)
    .fetch_one(&mut *tx)
    .await?;
    if !owned {
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(None);
    }
    let rows = sqlx::query_as::<_, Package>(
        "SELECT p.id,p.snapshot_id,p.state,
         CASE WHEN p.manifest_hash IS NULL THEN NULL ELSE encode(p.manifest_hash,'hex') END AS manifest_hash,
         to_char(p.created_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS created_at
         FROM finance.package p
         JOIN finance.dossier_snapshot s
           ON (s.tenant_id,s.company_id,s.id)=(p.tenant_id,p.company_id,p.snapshot_id)
         WHERE p.tenant_id=$1 AND p.company_id=$2 AND s.dossier_id=$3 AND p.id>$4
         ORDER BY p.id LIMIT $5",
    )
    .bind(tenant)
    .bind(company)
    .bind(dossier)
    .bind(after)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(Some(rows))
}

pub async fn complete(
    pool: &PgPool,
    lease: Lease,
    artifacts: &[Artifact<'_>],
) -> Result<i64, WorkerError> {
    complete_inner(pool, lease, artifacts, false).await
}

pub async fn complete_verified(
    pool: &PgPool,
    lease: Lease,
    artifacts: &[Artifact<'_>],
) -> Result<i64, WorkerError> {
    complete_inner(pool, lease, artifacts, true).await
}

async fn complete_inner(
    pool: &PgPool,
    lease: Lease,
    artifacts: &[Artifact<'_>],
    verified: bool,
) -> Result<i64, WorkerError> {
    let Some((manifest, total_bytes)) = manifest_digest(artifacts) else {
        return Err(WorkerError::InvalidManifest);
    };
    let scope = lease.scope();
    let tenant: Uuid = scope.tenant.into();
    let company: Uuid = scope.company.into();
    let mut tx = pool.begin().await?;
    let role: bool = sqlx::query_scalar(
        "SELECT current_user='finance_job_worker' AND NOT rolsuper AND NOT rolbypassrls FROM pg_roles WHERE rolname=current_user",
    )
    .fetch_one(&mut *tx)
    .await?;
    if !role {
        return Err(WorkerError::Forbidden);
    }
    sqlx::query(
        "SELECT set_config('finance.tenant_id',$1,true),set_config('finance.company_id',$2,true)",
    )
    .bind(tenant.to_string())
    .bind(company.to_string())
    .execute(&mut *tx)
    .await?;
    let allowed: bool = sqlx::query_scalar("SELECT finance.worker_scope_allowed($1,$2)")
        .bind(tenant)
        .bind(company)
        .fetch_one(&mut *tx)
        .await?;
    if !allowed {
        return Err(WorkerError::Forbidden);
    }
    let query = if verified {
        "SELECT finance.complete_export_job_verified($1,$2,$3,$4,$5,$6,$7,$8,$9)"
    } else {
        "SELECT finance.complete_export_job($1,$2,$3,$4,$5,$6,$7,$8)"
    };
    let query = sqlx::query_scalar::<_, i64>(query)
        .bind(tenant)
        .bind(company)
        .bind(lease.job_id)
        .bind(lease.fence)
        .bind(lease.worker_id())
        .bind(manifest.as_slice())
        .bind(i32::try_from(artifacts.len()).map_err(|_| WorkerError::InvalidManifest)?)
        .bind(total_bytes);
    let result = if verified {
        use std::fmt::Write;
        let manifest = artifacts
            .iter()
            .map(|x| {
                let mut digest = String::with_capacity(64);
                for b in x.expected.as_bytes() {
                    write!(digest, "{b:02x}").expect("write to string");
                }
                serde_json::json!({"name":x.name,"bytes":x.bytes,"sha256":digest})
            })
            .collect::<Vec<_>>();
        query
            .bind(serde_json::Value::Array(manifest))
            .fetch_one(&mut *tx)
            .await
    } else {
        query.fetch_one(&mut *tx).await
    };
    let version = match result {
        Ok(version) => version,
        Err(sqlx::Error::Database(database)) if database.code().as_deref() == Some("P0002") => {
            return Err(WorkerError::NotExportJob);
        }
        Err(sqlx::Error::Database(database)) if database.code().as_deref() == Some("23514") => {
            return Err(WorkerError::StaleLease);
        }
        Err(error) => return Err(WorkerError::Database(error)),
    };
    tx.commit().await?;
    Ok(version)
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, sqlx::FromRow)]
pub struct ExportPackage {
    pub package_id: Uuid,
    pub snapshot_id: Uuid,
    pub template_id: Uuid,
    pub sha256: String,
    pub artifact_count: i32,
    pub artifact_bytes: i64,
}

pub async fn export_manifest(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    package: Uuid,
) -> Result<Option<String>, Error> {
    if dossier.is_nil() || package.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|error| match error {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(error) => Error::Database(error),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let digest: Option<String> =
        sqlx::query_scalar("SELECT finance.runtime_export_manifest($1,$2,$3,$4,$5)")
            .bind(t)
            .bind(c)
            .bind(a)
            .bind(dossier)
            .bind(package)
            .fetch_one(&mut *tx)
            .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(digest)
}

pub async fn download_export(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    package: Uuid,
) -> Result<Option<ExportPackage>, Error> {
    if dossier.is_nil() || package.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|error| match error {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(error) => Error::Database(error),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = sqlx::query_as::<_, ExportPackage>(
        "SELECT * FROM finance.runtime_download_export($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(dossier)
    .bind(package)
    .bind(command)
    .bind(correlation)
    .fetch_optional(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

#[derive(sqlx::FromRow)]
pub struct Bundle {
    pub job_id: Uuid,
    pub artifact_fence: i64,
    pub sha256: String,
    pub byte_count: i64,
}
pub async fn bundle(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    package: Uuid,
) -> Result<Option<Bundle>, Error> {
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row =
        sqlx::query_as::<_, Bundle>("SELECT * FROM finance.runtime_export_bundle($1,$2,$3,$4,$5)")
            .bind(t)
            .bind(c)
            .bind(a)
            .bind(dossier)
            .bind(package)
            .fetch_optional(&mut *tx)
            .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
pub fn read_bundle(
    root: &std::path::Path,
    scope: CompanyScope,
    bundle: &Bundle,
) -> Option<Vec<u8>> {
    use std::{fs, io::Read};
    if !(1..=104857600).contains(&bundle.byte_count) || bundle.artifact_fence < 1 {
        return None;
    }
    let mut path = root.to_path_buf();
    if !root.is_absolute() || root.canonicalize().ok()?.as_path() != root {
        return None;
    }
    for part in [
        scope.tenant.to_string(),
        scope.company.to_string(),
        bundle.job_id.to_string(),
        bundle.artifact_fence.to_string(),
    ] {
        let m = fs::symlink_metadata(&path).ok()?;
        if !m.is_dir() || m.file_type().is_symlink() {
            return None;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if m.permissions().mode() & 0o077 != 0 {
                return None;
            }
        }
        path.push(part);
    }
    let m = fs::symlink_metadata(&path).ok()?;
    if !m.is_dir() || m.file_type().is_symlink() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if m.permissions().mode() & 0o077 != 0 {
            return None;
        }
    }
    path.push("bundle.zip");
    let m = fs::symlink_metadata(&path).ok()?;
    if !m.is_file() || m.file_type().is_symlink() || m.len() != bundle.byte_count as u64 {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if m.nlink() != 1 || m.mode() & 0o077 != 0 {
            return None;
        }
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(bundle.byte_count as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() != bundle.byte_count as usize
        || !bytes.starts_with(b"PK\x03\x04")
        || format!("{:x}", Sha256::digest(&bytes)) != bundle.sha256
    {
        return None;
    }
    Some(bytes)
}
