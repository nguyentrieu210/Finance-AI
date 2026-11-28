//! Opt-in local Tesseract pipeline for CLEAN synthetic invoice PNGs only.
//! Unconfigured or unsupported providers never claim work or invent candidates.
use super::{Lease, RunFailure, job_failure};
use finance_core::jobs;
use finance_document_intelligence::{Kind, normalize::{Format, Outcome}};
use finance_extraction::{Field, State, rows::{OwnedValue, extract}};
use finance_ocr::tsv::parse_page;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{env, fs, path::PathBuf, process::Stdio, time::Duration};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, process::Command, time};
use uuid::Uuid;

pub(super) const EVENT: &str = "OCR_REQUESTED";
const MAX_TSV: u64 = 2 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Config {
    binary: PathBuf,
    tessdata: PathBuf,
    model_digest: [u8; 32],
    binary_digest: [u8; 32],
}

impl Config {
    pub(super) fn from_env() -> Result<Option<Self>, &'static str> {
        let Some(binary) = env::var_os("FINANCE_OCR_SYNTHETIC_TESSERACT").map(PathBuf::from) else {
            return Ok(None);
        };
        let tessdata = env::var_os("FINANCE_OCR_TESSDATA_DIR").map(PathBuf::from).ok_or("OCR traineddata directory missing")?;
        if !binary.is_absolute() || !binary.is_file() || !tessdata.is_absolute() || !tessdata.is_dir() {
            return Err("OCR executable or traineddata directory invalid");
        }
        let model = tessdata.join("vie.traineddata");
        let metadata = fs::metadata(&model).map_err(|_| "OCR traineddata missing")?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 100 * 1024 * 1024 {
            return Err("OCR traineddata size invalid");
        }
        let bytes = fs::read(model).map_err(|_| "OCR traineddata unreadable")?;
        let model_digest = Sha256::digest(bytes).into();
        let metadata = fs::metadata(&binary).map_err(|_| "OCR executable missing")?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 100 * 1024 * 1024 {
            return Err("OCR executable size invalid");
        }
        let binary_digest = Sha256::digest(fs::read(&binary).map_err(|_| "OCR executable unreadable")?).into();
        Ok(Some(Self { binary, tessdata, model_digest, binary_digest }))
    }
}

#[derive(sqlx::FromRow)]
struct Source {
    document_id: Uuid,
    revision_id: Uuid,
    generation: i64,
    source_digest: Vec<u8>,
    content: Vec<u8>,
}

fn db_failure(error: sqlx::Error) -> RunFailure {
    match error.as_database_error().and_then(|db| db.code()).as_deref() {
        Some("40001") => RunFailure::Stale,
        Some("42501" | "23514" | "23505" | "P0002") => RunFailure::Permanent,
        _ => RunFailure::Transient,
    }
}

async fn source(pool: &PgPool, lease: Lease) -> Result<Source, RunFailure> {
    let scope = lease.scope();
    let (t, c): (Uuid, Uuid) = (scope.tenant.into(), scope.company.into());
    let mut tx = pool.begin().await.map_err(db_failure)?;
    sqlx::query("SELECT set_config('finance.tenant_id',$1,true),set_config('finance.company_id',$2,true)")
        .bind(t.to_string()).bind(c.to_string()).execute(&mut *tx).await.map_err(db_failure)?;
    let source = sqlx::query_as::<_, Source>("SELECT * FROM finance.synthetic_invoice_ocr_source($1,$2,$3,$4,$5)")
        .bind(t).bind(c).bind(lease.job_id).bind(lease.fence).bind(lease.worker_id())
        .fetch_optional(&mut *tx).await.map_err(db_failure)?
        .ok_or(RunFailure::Permanent)?;
    tx.commit().await.map_err(db_failure)?;
    Ok(source)
}

fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), RunFailure> {
    if bytes.len() < 33 || bytes.len() > 25 * 1024 * 1024
        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || &bytes[8..16] != b"\x00\x00\x00\x0dIHDR"
    {
        return Err(RunFailure::Permanent);
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().map_err(|_| RunFailure::Permanent)?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().map_err(|_| RunFailure::Permanent)?);
    if !(1..=8192).contains(&width) || !(1..=8192).contains(&height)
        || u64::from(width) * u64::from(height) > 25_000_000
    {
        return Err(RunFailure::Permanent);
    }
    Ok((width, height))
}

async fn recognize(config: &Config, content: &[u8]) -> Result<String, RunFailure> {
    let mut child = Command::new(&config.binary)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("OMP_THREAD_LIMIT", "1")
        .args(["stdin", "stdout", "-l", "vie", "--psm", "6", "--tessdata-dir"])
        .arg(&config.tessdata)
        .arg("tsv")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| RunFailure::Permanent)?;
    let mut input = child.stdin.take().ok_or(RunFailure::Permanent)?;
    let mut output = child.stdout.take().ok_or(RunFailure::Permanent)?.take(MAX_TSV+1);
    let io = async {
        let write = async move {
            input.write_all(content).await.map_err(|_| RunFailure::Transient)?;
            input.shutdown().await.map_err(|_| RunFailure::Transient)
        };
        let read = async {
            let mut bytes = Vec::new();
            output.read_to_end(&mut bytes).await.map_err(|_| RunFailure::Transient)?;
            if bytes.len() as u64 > MAX_TSV {
                return Err(RunFailure::Permanent);
            }
            Ok(bytes)
        };
        let (_, bytes) = tokio::try_join!(write, read)?;
        if !child.wait().await.map_err(|_| RunFailure::Transient)?.success() {
            return Err(RunFailure::Permanent);
        }
        String::from_utf8(bytes).map_err(|_| RunFailure::Permanent)
    };
    time::timeout(Duration::from_secs(60), io).await.map_err(|_| RunFailure::Transient)?
}

fn candidates(tsv: &str, width: u32, height: u32) -> Result<(Value, Value), RunFailure> {
    let page = parse_page(tsv, 1, 1, width, height).map_err(|_| RunFailure::Permanent)?;
    let blocks = Value::Array(page.blocks().iter().map(|b| json!({
        "rect":b.rect().coordinates(),"text":b.text(),"confidence":b.confidence()
    })).collect());
    let results = extract(Kind::Invoice, std::slice::from_ref(&page), Format::Vietnamese)
        .map_err(|_| RunFailure::Permanent)?;
    let mut fields = Vec::new();
    for field in results {
        let path = match field.field {
            Field::InvoiceNumber => "invoice.number",
            Field::DocumentDate => "invoice.date",
            Field::Supplier => "invoice.supplier",
            Field::AmountBeforeTax => "invoice.net",
            Field::TaxAmount => "invoice.tax",
            Field::TotalAmount => "invoice.total",
            _ => return Err(RunFailure::Permanent),
        };
        let state = match field.state {
            State::Unknown => "UNKNOWN",
            State::Invalid => "INVALID",
            State::Ambiguous => "AMBIGUOUS",
            State::Candidate => "CANDIDATE",
        };
        let mut value = None::<String>;
        let mut indexes = Vec::new();
        let mut source_page = None;
        if field.state == State::Candidate {
            let obs = field.observations.first().ok_or(RunFailure::Permanent)?;
            let Outcome::Candidate(v) = &obs.outcome else {
                return Err(RunFailure::Permanent);
            };
            value = Some(match v {
                OwnedValue::Amount(a) => a.decimal().to_string(),
                OwnedValue::Date(d) => d.to_string(),
                OwnedValue::Identifier(s) | OwnedValue::Text(s) => s.clone(),
            });
            indexes = obs.block_indexes.clone();
            source_page = Some(obs.page);
        }
        fields.push(json!({"path":path,"state":state,"value":value,"page":source_page,"indexes":indexes}));
    }
    if fields.len() != 6 {
        return Err(RunFailure::Permanent);
    }
    Ok((blocks, Value::Array(fields)))
}

pub(super) async fn process(pool: &PgPool, config: &Config, lease: Lease, lease_seconds: i32) -> Result<(), RunFailure> {
    let source = source(pool, lease).await?;
    if Sha256::digest(&source.content).as_slice() != source.source_digest.as_slice() {
        return Err(RunFailure::Permanent);
    }
    let (width,height) = png_dimensions(&source.content)?;
    let mut work = Box::pin(recognize(config, &source.content));
    let mut heartbeat = time::interval(Duration::from_secs((lease_seconds/3).max(1) as u64));
    heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let tsv = loop {
        tokio::select! {
            result = &mut work => break result?,
            _ = heartbeat.tick() => jobs::heartbeat(pool,lease,lease_seconds).await.map_err(job_failure)?,
        }
    };
    let (blocks, fields) = candidates(&tsv,width,height)?;
    let mut configuration = Sha256::new();
    configuration.update(b"finance.synthetic.ocr/v1");
    configuration.update(config.binary_digest);
    configuration.update(format!("tesseract/vie/psm6/{width}x{height}/rows.v1"));
    let config_digest = configuration.finalize();
    let scope = lease.scope();
    let (t,c): (Uuid,Uuid) = (scope.tenant.into(),scope.company.into());
    let mut tx = pool.begin().await.map_err(db_failure)?;
    sqlx::query("SELECT set_config('finance.tenant_id',$1,true),set_config('finance.company_id',$2,true)")
        .bind(t.to_string()).bind(c.to_string()).execute(&mut *tx).await.map_err(db_failure)?;
    sqlx::query_scalar::<_,Uuid>("SELECT finance.complete_synthetic_invoice_ocr($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)")
        .bind(t).bind(c).bind(lease.job_id).bind(lease.fence).bind(lease.worker_id())
        .bind(source.document_id).bind(source.revision_id).bind(source.generation)
        .bind(&source.source_digest).bind(config.model_digest.as_slice()).bind(config_digest.as_slice())
        .bind(blocks).bind(fields).fetch_one(&mut *tx).await.map_err(db_failure)?;
    tx.commit().await.map_err(db_failure)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_png_before_ocr_and_persists_only_six_candidates() {
        let mut png = vec![0_u8;33];
        png[..16].copy_from_slice(b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR");
        png[16..20].copy_from_slice(&100_u32.to_be_bytes());
        png[20..24].copy_from_slice(&100_u32.to_be_bytes());
        assert_eq!(png_dimensions(&png),Ok((100,100)));
        png[16..20].copy_from_slice(&100_000_u32.to_be_bytes());
        assert_eq!(png_dimensions(&png),Err(RunFailure::Permanent));
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n1\t1\t0\t0\t0\t0\t0\t0\t100\t100\t-1\t\n";
        let (_,fields) = candidates(tsv,100,100).unwrap();
        assert_eq!(fields.as_array().unwrap().len(),6);
        assert!(fields.as_array().unwrap().iter().all(|f| f["state"]=="UNKNOWN"));
    }
}
