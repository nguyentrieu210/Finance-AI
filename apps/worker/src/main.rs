#![forbid(unsafe_code)]
mod artifact_store;
mod ocr;
use finance_core::jobs::{self, Failure, FailureResult, JobError, Lease};
use finance_shared::{
    ids::{CompanyId, CompanyScope, TenantId},
    upload_contract::Sha256Digest,
};
use finance_templates::{Artifact, export, verified_manifest};
use serde::Deserialize;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    env,
    path::PathBuf,
    process::{ExitCode, Stdio},
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    process::Command,
    time::{self, MissedTickBehavior},
};
use uuid::Uuid;

const EXPORT_EVENT: &str = "EXPORT_REQUESTED";
const PREFLIGHT_EVENT: &str = "PREFLIGHT_REQUESTED";
const RECEIPT_SCHEMA: &str = "finance.export.receipt.v1";
const REQUEST_SCHEMA: &str = "finance.export.request.v1";
const MAX_RECEIPT_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
struct Config {
    database_url: String,
    scope: CompanyScope,
    worker: Uuid,
    lease_seconds: i32,
    poll: Duration,
    adapter_timeout: Duration,
    adapter: PathBuf,
    adapter_config: Option<PathBuf>,
    adapter_endpoint: Option<String>,
    artifact_root: Option<PathBuf>,
    ocr: Option<ocr::Config>,
}

impl Config {
    fn parse() -> Result<(Self, bool), &'static str> {
        let args: Vec<String> = env::args().skip(1).collect();
        let once = match args.as_slice() {
            [] => false,
            [value] if value == "--once" => true,
            _ => return Err("unsupported worker arguments"),
        };
        let database_url =
            env::var("FINANCE_WORKER_DATABASE_URL").map_err(|_| "worker database missing")?;
        if database_url.len() > 8192 {
            return Err("worker database configuration invalid");
        }
        let tenant = parse_uuid("FINANCE_WORKER_TENANT_ID")?;
        let company = parse_uuid("FINANCE_WORKER_COMPANY_ID")?;
        let worker = parse_uuid("FINANCE_WORKER_ID")?;
        let scope = CompanyScope {
            tenant: TenantId::try_from(tenant).map_err(|_| "worker tenant invalid")?,
            company: CompanyId::try_from(company).map_err(|_| "worker company invalid")?,
        };
        let lease_seconds = parse_i32("FINANCE_WORKER_LEASE_SECONDS", 60, 3, 300)?;
        let poll_ms = parse_u64("FINANCE_WORKER_POLL_MS", 1000, 100, 60_000)?;
        let timeout_seconds = parse_u64("FINANCE_EXPORT_ADAPTER_TIMEOUT_SECONDS", 120, 1, 300)?;
        let adapter = PathBuf::from(
            env::var("FINANCE_EXPORT_ADAPTER").map_err(|_| "export adapter missing")?,
        );
        if !adapter.is_absolute() {
            return Err("export adapter path must be absolute");
        }
        let adapter_config = env::var_os("FINANCE_EXPORT_ADAPTER_CONFIG").map(PathBuf::from);
        if adapter_config
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
        {
            return Err("export adapter config path must be absolute");
        }
        let adapter_endpoint = env::var("FINANCE_EXPORT_STORAGE_ENDPOINT").ok();
        if adapter_endpoint
            .as_ref()
            .is_some_and(|value| value.len() > 2048)
        {
            return Err("export adapter endpoint invalid");
        }
        let artifact_root = env::var_os("FINANCE_EXPORT_ARTIFACT_ROOT").map(PathBuf::from);
        let synthetic_receipt_only = env::var("FINANCE_EXPORT_SYNTHETIC_RECEIPTS_ONLY").as_deref() == Ok("1");
        if artifact_root.is_none() && !synthetic_receipt_only {
            return Err("independent export artifact readback configuration missing");
        }
        if artifact_root.as_ref().is_some_and(|path| !artifact_store::valid_root(path)) {
            return Err("export artifact root must be an existing private absolute directory");
        }
        if artifact_root.is_some() && synthetic_receipt_only {
            return Err("export artifact verification mode is ambiguous");
        }
        Ok((
            Self {
                database_url,
                scope,
                worker,
                lease_seconds,
                poll: Duration::from_millis(poll_ms),
                adapter_timeout: Duration::from_secs(timeout_seconds),
                adapter,
                adapter_config,
                adapter_endpoint,
                artifact_root,
                ocr: ocr::Config::from_env()?,
            },
            once,
        ))
    }
}

fn parse_uuid(name: &'static str) -> Result<Uuid, &'static str> {
    let value = env::var(name).map_err(|_| "worker identity configuration missing")?;
    let id = Uuid::parse_str(&value).map_err(|_| "worker identity configuration invalid")?;
    if id.is_nil() {
        return Err("worker identity configuration invalid");
    }
    Ok(id)
}

fn parse_i32(name: &'static str, default: i32, min: i32, max: i32) -> Result<i32, &'static str> {
    let value = match env::var(name) {
        Ok(value) => value
            .parse::<i32>()
            .map_err(|_| "worker numeric configuration invalid")?,
        Err(_) => default,
    };
    if !(min..=max).contains(&value) {
        return Err("worker numeric configuration out of range");
    }
    Ok(value)
}

fn parse_u64(name: &'static str, default: u64, min: u64, max: u64) -> Result<u64, &'static str> {
    let value = match env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| "worker numeric configuration invalid")?,
        Err(_) => default,
    };
    if !(min..=max).contains(&value) {
        return Err("worker numeric configuration out of range");
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum RunFailure {
    Transient,
    Permanent,
    Stale,
}

fn job_failure(error: JobError) -> RunFailure {
    match error {
        JobError::StaleLease => RunFailure::Stale,
        JobError::Database(_) => RunFailure::Transient,
        JobError::Forbidden | JobError::InvalidInput | JobError::Invariant => RunFailure::Permanent,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: String,
    job_id: Uuid,
    fence: i64,
    artifacts: Vec<ReceiptArtifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptArtifact {
    name: String,
    expected_sha256: String,
    observed_sha256: String,
    bytes: u64,
}

#[derive(Debug)]
struct OwnedArtifact {
    name: String,
    expected: Sha256Digest,
    observed: Sha256Digest,
    bytes: u64,
}

fn validate_receipt(
    receipt: Receipt,
    expected_job: Uuid,
    expected_fence: i64,
) -> Result<Vec<OwnedArtifact>, RunFailure> {
    if receipt.schema != RECEIPT_SCHEMA
        || receipt.job_id != expected_job
        || receipt.fence != expected_fence
        || receipt.artifacts.is_empty()
        || receipt.artifacts.len() > 256
    {
        return Err(RunFailure::Permanent);
    }
    let owned: Result<Vec<_>, _> = receipt
        .artifacts
        .into_iter()
        .map(|item| {
            let expected =
                Sha256Digest::parse(&item.expected_sha256).map_err(|_| RunFailure::Permanent)?;
            let observed =
                Sha256Digest::parse(&item.observed_sha256).map_err(|_| RunFailure::Permanent)?;
            Ok(OwnedArtifact {
                name: item.name,
                expected,
                observed,
                bytes: item.bytes,
            })
        })
        .collect();
    let owned = owned?;
    let artifacts: Vec<_> = owned
        .iter()
        .map(|item| Artifact {
            name: &item.name,
            expected: item.expected,
            observed: Some(item.observed),
            bytes: item.bytes,
        })
        .collect();
    if !verified_manifest(&artifacts) {
        return Err(RunFailure::Permanent);
    }
    Ok(owned)
}

async fn invoke_adapter(config: &Config, lease: Lease) -> Result<Receipt, RunFailure> {
    let scope = lease.scope();
    let mut command = Command::new(&config.adapter);
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .arg("--schema")
        .arg(REQUEST_SCHEMA)
        .arg("--job-id")
        .arg(lease.job_id.to_string())
        .arg("--tenant-id")
        .arg(Uuid::from(scope.tenant).to_string())
        .arg("--company-id")
        .arg(Uuid::from(scope.company).to_string())
        .arg("--fence")
        .arg(lease.fence.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(path) = &config.adapter_config {
        command.env("FINANCE_EXPORT_ADAPTER_CONFIG", path);
    }
    if let Some(endpoint) = &config.adapter_endpoint {
        command.env("FINANCE_EXPORT_STORAGE_ENDPOINT", endpoint);
    }
    if let Some(root) = &config.artifact_root {
        command.env("FINANCE_EXPORT_ARTIFACT_ROOT", root);
    }
    let mut child = command.spawn().map_err(|_| RunFailure::Permanent)?;
    let stdout = child.stdout.take().ok_or(RunFailure::Permanent)?;
    let operation = async move {
        let mut bytes = Vec::new();
        let mut bounded = stdout.take((MAX_RECEIPT_BYTES + 1) as u64);
        bounded
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| RunFailure::Transient)?;
        if bytes.len() > MAX_RECEIPT_BYTES {
            return Err(RunFailure::Permanent);
        }
        let status = child.wait().await.map_err(|_| RunFailure::Transient)?;
        if !status.success() {
            return Err(if status.code() == Some(75) {
                RunFailure::Transient
            } else {
                RunFailure::Permanent
            });
        }
        serde_json::from_slice::<Receipt>(&bytes).map_err(|_| RunFailure::Permanent)
    };
    time::timeout(config.adapter_timeout, operation)
        .await
        .map_err(|_| RunFailure::Transient)?
}

async fn render_with_heartbeat(
    pool: &PgPool,
    config: &Config,
    lease: Lease,
) -> Result<Receipt, RunFailure> {
    let mut adapter = Box::pin(invoke_adapter(config, lease));
    let interval_seconds =
        u64::try_from((config.lease_seconds / 3).max(1)).map_err(|_| RunFailure::Permanent)?;
    let mut heartbeat = time::interval(Duration::from_secs(interval_seconds));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    heartbeat.tick().await;
    loop {
        tokio::select! {
            result = &mut adapter => return result,
            _ = heartbeat.tick() => {
                jobs::heartbeat(pool, lease, config.lease_seconds)
                    .await
                    .map_err(job_failure)?;
            }
        }
    }
}

async fn process_export(pool: &PgPool, config: &Config, lease: Lease) -> Result<(), RunFailure> {
    let receipt = render_with_heartbeat(pool, config, lease).await?;
    let mut owned = validate_receipt(receipt, lease.job_id, lease.fence)?;
    if let Some(root) = &config.artifact_root {
        let root = root.clone();
        owned = tokio::task::spawn_blocking(move || {
            artifact_store::verify(&root, lease, &owned)?;
            Ok::<_, RunFailure>(owned)
        })
        .await
        .map_err(|_| RunFailure::Transient)??;
        jobs::heartbeat(pool, lease, config.lease_seconds)
            .await
            .map_err(job_failure)?;
    }
    let artifacts: Vec<_> = owned
        .iter()
        .map(|item| Artifact {
            name: &item.name,
            expected: item.expected,
            observed: Some(item.observed),
            bytes: item.bytes,
        })
        .collect();
    export::complete(pool, lease, &artifacts)
        .await
        .map(|_| ())
        .map_err(|error| match error {
            export::WorkerError::StaleLease => RunFailure::Stale,
            export::WorkerError::Database(_) => RunFailure::Transient,
            export::WorkerError::InvalidManifest
            | export::WorkerError::NotExportJob
            | export::WorkerError::Forbidden => RunFailure::Permanent,
        })
}

async fn settle_failure(pool: &PgPool, lease: Lease, failure: RunFailure) {
    let kind = match failure {
        RunFailure::Transient => Some(Failure::Transient),
        RunFailure::Permanent => Some(Failure::Permanent),
        RunFailure::Stale => None,
    };
    let Some(kind) = kind else {
        return;
    };
    match jobs::fail(pool, lease, kind).await {
        Ok(FailureResult::Retry) => eprintln!("finance-worker: job retry scheduled"),
        Ok(FailureResult::Dead) => eprintln!("finance-worker: job moved to dead letter"),
        Err(JobError::StaleLease) => eprintln!("finance-worker: stale job failure ignored"),
        Err(_) => eprintln!("finance-worker: failed to persist job failure"),
    }
}

async fn process_preflight(pool: &PgPool, lease: Lease) -> Result<(), RunFailure> {
    let scope = lease.scope();
    let mut tx = pool.begin().await.map_err(|_| RunFailure::Transient)?;
    let (t, c): (Uuid, Uuid) = (scope.tenant.into(), scope.company.into());
    sqlx::query(
        "SELECT set_config('finance.tenant_id',$1,true),set_config('finance.company_id',$2,true)",
    )
    .bind(t.to_string())
    .bind(c.to_string())
    .execute(&mut *tx)
    .await
    .map_err(|_| RunFailure::Transient)?;
    sqlx::query("SELECT finance.complete_preflight_job($1,$2,$3,$4,$5)")
        .bind(t)
        .bind(c)
        .bind(lease.job_id)
        .bind(lease.fence)
        .bind(lease.worker_id())
        .execute(&mut *tx)
        .await
        .map_err(
            |e| match e.as_database_error().and_then(|db| db.code()).as_deref() {
                Some("40001") => RunFailure::Stale,
                Some("42501" | "23514" | "23505" | "P0002") => RunFailure::Permanent,
                _ => RunFailure::Transient,
            },
        )?;
    tx.commit().await.map_err(|_| RunFailure::Transient)?;
    Ok(())
}

async fn cycle(pool: &PgPool, config: &Config) -> Result<bool, RunFailure> {
    jobs::dispatch_event(pool, config.scope, PREFLIGHT_EVENT, 100)
        .await
        .map_err(job_failure)?;
    if let Some(lease) = jobs::claim_event(
        pool,
        config.scope,
        config.worker,
        config.lease_seconds,
        PREFLIGHT_EVENT,
    )
    .await
    .map_err(job_failure)?
    {
        return match process_preflight(pool, lease).await {
            Ok(()) => Ok(true),
            Err(failure) => {
                settle_failure(pool, lease, failure).await;
                Err(failure)
            }
        };
    }
    if let Some(ocr_config) = &config.ocr {
        jobs::dispatch_event(pool, config.scope, ocr::EVENT, 100)
            .await
            .map_err(job_failure)?;
        if let Some(lease) = jobs::claim_event(
            pool,
            config.scope,
            config.worker,
            config.lease_seconds,
            ocr::EVENT,
        )
        .await
        .map_err(job_failure)?
        {
            return match ocr::process(pool, ocr_config, lease, config.lease_seconds).await {
                Ok(()) => Ok(true),
                Err(failure) => {
                    settle_failure(pool, lease, failure).await;
                    Err(failure)
                }
            };
        }
    }
    jobs::dispatch_event(pool, config.scope, EXPORT_EVENT, 100)
        .await
        .map_err(job_failure)?;
    let Some(lease) = jobs::claim_event(
        pool,
        config.scope,
        config.worker,
        config.lease_seconds,
        EXPORT_EVENT,
    )
    .await
    .map_err(job_failure)?
    else {
        return Ok(false);
    };
    match process_export(pool, config, lease).await {
        Ok(()) => Ok(true),
        Err(failure) => {
            settle_failure(pool, lease, failure).await;
            Err(failure)
        }
    }
}

async fn connect(config: &Config) -> Result<PgPool, &'static str> {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE finance_job_worker")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect(&config.database_url)
        .await
        .map_err(|_| "worker database unavailable")
}

#[tokio::main]
async fn main() -> ExitCode {
    let (config, once) = match Config::parse() {
        Ok(value) => value,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(78);
        }
    };
    let pool = match connect(&config).await {
        Ok(pool) => pool,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(69);
        }
    };
    if once {
        let result = cycle(&pool, &config).await;
        pool.close().await;
        return match result {
            Ok(_) => ExitCode::SUCCESS,
            Err(RunFailure::Transient) => ExitCode::from(75),
            Err(RunFailure::Permanent) => ExitCode::from(65),
            Err(RunFailure::Stale) => ExitCode::from(70),
        };
    }
    loop {
        match cycle(&pool, &config).await {
            Ok(true) => {}
            Ok(false) => time::sleep(config.poll).await,
            Err(RunFailure::Transient) => {
                eprintln!("finance-worker: transient cycle failure");
                time::sleep(config.poll).await;
            }
            Err(RunFailure::Permanent) => {
                eprintln!("finance-worker: permanent job failure recorded");
            }
            Err(RunFailure::Stale) => {
                eprintln!("finance-worker: stale lease discarded");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest() -> String {
        "a".repeat(64)
    }

    #[test]
    fn receipt_is_bound_to_job_fence_and_storage_readback() {
        let job = Uuid::from_u128(11);
        let base = Receipt {
            schema: RECEIPT_SCHEMA.into(),
            job_id: job,
            fence: 2,
            artifacts: vec![ReceiptArtifact {
                name: "request.pdf".into(),
                expected_sha256: digest(),
                observed_sha256: digest(),
                bytes: 100,
            }],
        };
        assert_eq!(validate_receipt(base, job, 2).unwrap().len(), 1);
        let mismatch = Receipt {
            schema: RECEIPT_SCHEMA.into(),
            job_id: job,
            fence: 3,
            artifacts: vec![ReceiptArtifact {
                name: "request.pdf".into(),
                expected_sha256: digest(),
                observed_sha256: "b".repeat(64),
                bytes: 100,
            }],
        };
        assert_eq!(
            validate_receipt(mismatch, job, 2).unwrap_err(),
            RunFailure::Permanent
        );
    }
}
