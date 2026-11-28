//! Version-bound external document malware scan with fenced atomic completion.
use super::{Lease, RunFailure, job_failure};
use finance_core::jobs;
use finance_documents::{
    Availability, Binding, Original,
    s3::{Object, Phase, S3},
    scan::{ClamdScanner, ScanVerdict},
};
use finance_shared::{
    contracts::Version,
    ids::{ResourceId, RevisionId},
    upload_contract::{DeclaredMime, Sha256Digest},
};
use sqlx::PgPool;
use std::{env, net::SocketAddr, time::Duration};
use tokio::time;
use uuid::Uuid;

pub(super) const EVENT: &str = "DOCUMENT_FINALIZE_REQUESTED";
#[derive(Clone)]
pub(super) struct Config {
    storage: S3,
    address: SocketAddr,
    engine_digest: String,
}

impl Config {
    pub(super) fn from_env() -> Result<Option<Self>, &'static str> {
        let Some(storage) = S3::from_env().map_err(|_| "S3 configuration invalid")? else {
            return Ok(None);
        };
        let address = env::var("FINANCE_CLAMD_ADDRESS")
            .map_err(|_| "ClamAV address missing")?
            .parse()
            .map_err(|_| "ClamAV address invalid")?;
        ClamdScanner::new(address, Duration::from_secs(60))
            .map_err(|_| "ClamAV address invalid")?;
        let engine_digest = env::var("FINANCE_CLAMD_ENGINE_SHA256")
            .map_err(|_| "ClamAV engine identity missing")?;
        Sha256Digest::parse(&engine_digest).map_err(|_| "ClamAV engine identity invalid")?;
        Ok(Some(Self {
            storage,
            address,
            engine_digest,
        }))
    }
}

#[derive(sqlx::FromRow)]
struct Source {
    document_id: Uuid,
    revision_id: Uuid,
    generation: i64,
    byte_count: i64,
    mime: String,
    digest: String,
    object_version: String,
}

fn db_failure(error: sqlx::Error) -> RunFailure {
    match error.as_database_error().and_then(|e| e.code()).as_deref() {
        Some("40001") => RunFailure::Stale,
        Some("42501" | "23514" | "23505") => RunFailure::Permanent,
        _ => RunFailure::Transient,
    }
}

async fn source(pool: &PgPool, lease: Lease) -> Result<Source, RunFailure> {
    let scope = lease.scope();
    let (t, c): (Uuid, Uuid) = (scope.tenant.into(), scope.company.into());
    let mut tx = pool.begin().await.map_err(db_failure)?;
    sqlx::query(
        "SELECT set_config('finance.tenant_id',$1,true),set_config('finance.company_id',$2,true)",
    )
    .bind(t.to_string())
    .bind(c.to_string())
    .execute(&mut *tx)
    .await
    .map_err(db_failure)?;
    let row=sqlx::query_as::<_,Source>("SELECT b.id AS document_id,r.revision_id,b.generation,b.byte_count,b.mime,encode(b.sha256_digest,'hex') AS digest,b.object_version FROM finance.job q JOIN finance.outbox o ON (o.tenant_id,o.company_id,o.id)=(q.tenant_id,q.company_id,q.id) JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(o.tenant_id,o.company_id,o.resource_id) JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(b.tenant_id,b.company_id,b.id) WHERE (q.tenant_id,q.company_id,q.id,q.fence,q.leased_by)=($1,$2,$3,$4,$5) AND q.state='LEASED' AND q.lease_until>clock_timestamp() AND o.event_type='DOCUMENT_FINALIZE_REQUESTED' AND b.storage_backend='S3_VERSIONED' AND b.scan_state='QUARANTINED'")
        .bind(t).bind(c).bind(lease.job_id).bind(lease.fence).bind(lease.worker_id()).fetch_optional(&mut *tx).await.map_err(db_failure)?
        .ok_or(RunFailure::Permanent)?;
    tx.commit().await.map_err(db_failure)?;
    Ok(row)
}

async fn scan(config: &Config, lease: Lease, source: &Source) -> Result<ScanVerdict, RunFailure> {
    let original = Original {
        binding: Binding {
            scope: lease.scope(),
            document: ResourceId::try_from(source.document_id)
                .map_err(|_| RunFailure::Permanent)?,
            revision: RevisionId::try_from(source.revision_id)
                .map_err(|_| RunFailure::Permanent)?,
        },
        generation: Version::try_from(source.generation).map_err(|_| RunFailure::Permanent)?,
        bytes: u64::try_from(source.byte_count).map_err(|_| RunFailure::Permanent)?,
        digest: Sha256Digest::parse(&source.digest).map_err(|_| RunFailure::Permanent)?,
        mime: DeclaredMime::parse(&source.mime).map_err(|_| RunFailure::Permanent)?,
        availability: Availability::Quarantined,
    };
    let object = Object {
        binding: original.binding,
        generation: original.generation,
        size: original.bytes,
        mime: original.mime,
        digest: original.digest,
    };
    let bytes = config
        .storage
        .read_verified(
            &object,
            Phase::Quarantine,
            &source.object_version,
            chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now()),
        )
        .await
        .map_err(|error| match error {
            finance_documents::s3::Error::Integrity
            | finance_documents::s3::Error::Metadata
            | finance_documents::s3::Error::InvalidVersion => RunFailure::Permanent,
            _ => RunFailure::Transient,
        })?;
    let address = config.address;
    tokio::task::spawn_blocking(move || {
        let scanner = ClamdScanner::new(address, Duration::from_secs(60))
            .map_err(|_| RunFailure::Permanent)?;
        let receipt = scanner
            .scan(original, &bytes)
            .map_err(|_| RunFailure::Transient)?;
        match receipt.verdict() {
            ScanVerdict::Indeterminate => Err(RunFailure::Transient),
            verdict => Ok(verdict),
        }
    })
    .await
    .map_err(|_| RunFailure::Transient)?
}

pub(super) async fn process(
    pool: &PgPool,
    config: &Config,
    lease: Lease,
    lease_seconds: i32,
) -> Result<(), RunFailure> {
    let source = source(pool, lease).await?;
    let mut work = Box::pin(scan(config, lease, &source));
    let mut heartbeat = time::interval(Duration::from_secs((lease_seconds / 3).max(1) as u64));
    heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let verdict = loop {
        tokio::select! {
            result=&mut work=>break result?,
            _=heartbeat.tick()=>jobs::heartbeat(pool,lease,lease_seconds).await.map_err(job_failure)?,
        }
    };
    let state = match verdict {
        ScanVerdict::NoSignatureMatch => "CLEAN",
        ScanVerdict::SignatureDetected => "REJECTED",
        ScanVerdict::Indeterminate => return Err(RunFailure::Transient),
    };
    let scope = lease.scope();
    let (t, c): (Uuid, Uuid) = (scope.tenant.into(), scope.company.into());
    let mut tx = pool.begin().await.map_err(db_failure)?;
    sqlx::query(
        "SELECT set_config('finance.tenant_id',$1,true),set_config('finance.company_id',$2,true)",
    )
    .bind(t.to_string())
    .bind(c.to_string())
    .execute(&mut *tx)
    .await
    .map_err(db_failure)?;
    sqlx::query("SELECT finance.complete_s3_scan($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
        .bind(t)
        .bind(c)
        .bind(lease.job_id)
        .bind(lease.fence)
        .bind(lease.worker_id())
        .bind(source.document_id)
        .bind(source.generation)
        .bind(&source.object_version)
        .bind(&source.digest)
        .bind(state)
        .bind(&config.engine_digest)
        .execute(&mut *tx)
        .await
        .map_err(db_failure)?;
    tx.commit().await.map_err(db_failure)?;
    Ok(())
}
