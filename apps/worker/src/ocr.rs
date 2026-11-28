//! Opt-in bounded local OCR pipeline for verified CLEAN PNG/PDF originals.
//! Unconfigured or unsupported providers never claim work or invent candidates.
use super::{Lease, RunFailure, job_failure};
use finance_core::jobs;
use finance_document_intelligence::{Kind, Decision, classify, normalize::{Format, Outcome}};
use finance_documents::{Binding, evidence::Rotation, s3::{S3,Object,Phase}};
use finance_shared::{contracts::Version,ids::{ResourceId,RevisionId},upload_contract::{DeclaredMime,Sha256Digest}};
use finance_ocr::PageResult;
use finance_extraction::{Field, State, rows::{OwnedValue, extract}};
use finance_ocr::tsv::parse_page;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{env, fs, path::PathBuf, process::Stdio, time::Duration};
mod pdf;
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, process::Command, time};
use uuid::Uuid;

pub(super) const EVENTS: [&str; 3] = ["OCR_REQUESTED", "CLASSIFY_REQUESTED", "EXTRACT_REQUESTED"];
const MAX_TSV: u64 = 2 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Config {
    binary: PathBuf,
    tessdata: PathBuf,
    model_digest: [u8; 32],
    binary_digest: [u8; 32],
    storage: Option<S3>,
    pdf: Option<pdf::Config>,
}

impl Config {
    pub(super) fn from_env() -> Result<Option<Self>, &'static str> {
        let external = env::var_os("FINANCE_OCR_TESSERACT");
        let storage = if external.is_some() {
            Some(S3::from_env().map_err(|_| "OCR storage invalid")?.ok_or("OCR external storage disabled")?)
        } else { None };
        let Some(binary) = external.or_else(|| env::var_os("FINANCE_OCR_SYNTHETIC_TESSERACT")).map(PathBuf::from) else {
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
        let pdf = pdf::Config::from_env()?;
        Ok(Some(Self { binary, tessdata, model_digest, binary_digest, storage, pdf }))
    }
}

#[derive(sqlx::FromRow)]
struct Source {
    document_id: Uuid,
    revision_id: Uuid,
    generation: i64,
    source_digest: Vec<u8>,
    content: Option<Vec<u8>>,
    object_version: Option<String>,
    byte_count: i64,
    mime: String,
    kind: String,
    event_type: String,
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
    let source = sqlx::query_as::<_, Source>("SELECT * FROM finance.local_ocr_source($1,$2,$3,$4,$5)")
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

fn candidates_for(kind: Kind,pages: &[PageResult], rotations: &[Rotation]) -> Result<(Value, Value, Value), RunFailure> {
    if pages.is_empty() || pages.len()!=rotations.len() { return Err(RunFailure::Permanent); }
    let mut blocks=Vec::new();
    let mut classifications=Vec::new();
    for (page,rotation) in pages.iter().zip(rotations) {
        for (index,b) in page.blocks().iter().enumerate() {
            if blocks.len()>=100_000 { return Err(RunFailure::Permanent); }
            let inverse=match rotation { Rotation::R0=>Rotation::R0,Rotation::R90=>Rotation::R270,Rotation::R180=>Rotation::R180,Rotation::R270=>Rotation::R90 };
            blocks.push(json!({"page":page.page(),"index":index,"rect":b.rect().rotate(inverse).coordinates(),"text":b.text(),"confidence":b.confidence()}));
        }
        let result=classify(page);
        let (state,candidate)=match result.decision {Decision::Unknown=>("UNKNOWN",None),Decision::Ambiguous=>("AMBIGUOUS",None),Decision::Candidate(k)=>("CANDIDATE",Some(kind_name(k)))};
        let hits:Vec<_>=result.hits.iter().map(|hit|json!({"kind":kind_name(hit.kind),"indexes":hit.block_indexes})).collect();
        classifications.push(json!({"page":page.page(),"state":state,"candidate":candidate,"hits":hits}));
    }
    let results = extract(kind, pages, Format::Vietnamese)
        .map_err(|_| RunFailure::Permanent)?;
    let mut fields = Vec::new();
    for field in results {
        let prefix=kind_prefix(kind);
        let suffix=match field.field {
            Field::ContractNumber if matches!(kind,Kind::Contract|Kind::ContractAppendix)=>"number",
            Field::ContractNumber=>"contract_number",
            Field::InvoiceNumber if kind==Kind::Invoice=>"number",
            Field::InvoiceNumber=>"invoice_number",
            Field::DocumentDate=>"date",
            Field::Supplier=>"supplier",
            Field::AmountBeforeTax=>"net",
            Field::TaxAmount=>"tax",
            Field::TotalAmount=>"total",
            Field::RequestedAmount=>"requested",
        };
        let path=format!("{prefix}.{suffix}");
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
    if fields.is_empty() || fields.len()>16 { return Err(RunFailure::Permanent); }
    Ok((Value::Array(blocks), Value::Array(fields), Value::Array(classifications)))
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Contract=>"CONTRACT", Kind::ContractAppendix=>"CONTRACT_APPENDIX",Kind::Invoice=>"INVOICE",
        Kind::PaymentRequest=>"PAYMENT_REQUEST",Kind::AdvanceRequest=>"ADVANCE_REQUEST",Kind::AdvanceSettlement=>"ADVANCE_SETTLEMENT",
        Kind::AcceptanceRecord=>"ACCEPTANCE_RECORD",Kind::PaymentVoucher=>"PAYMENT_VOUCHER",
    }
}

fn kind_prefix(kind: Kind) -> &'static str {
    match kind {
        Kind::Contract=>"contract",Kind::ContractAppendix=>"contract_appendix",Kind::Invoice=>"invoice",
        Kind::PaymentRequest=>"payment_request",Kind::AdvanceRequest=>"advance_request",Kind::AdvanceSettlement=>"advance_settlement",
        Kind::AcceptanceRecord=>"acceptance_record",Kind::PaymentVoucher=>"payment_voucher",
    }
}

fn parse_kind(value: &str) -> Result<Kind,RunFailure> {
    [Kind::Contract,Kind::ContractAppendix,Kind::Invoice,Kind::PaymentRequest,Kind::AdvanceRequest,Kind::AdvanceSettlement,Kind::AcceptanceRecord,Kind::PaymentVoucher]
        .into_iter().find(|kind|kind_name(*kind)==value).ok_or(RunFailure::Permanent)
}

async fn input_bytes(config:&Config,lease:Lease,source:&Source)->Result<Vec<u8>,RunFailure>{
    if let Some(content)=&source.content {
        if source.object_version.is_some() || Sha256::digest(content).as_slice()!=source.source_digest.as_slice() {
            return Err(RunFailure::Permanent);
        }
        return Ok(content.clone());
    }
    let storage=config.storage.as_ref().ok_or(RunFailure::Permanent)?;
    let version=source.object_version.as_deref().ok_or(RunFailure::Permanent)?;
    let hash=source.source_digest.iter().map(|b|format!("{b:02x}")).collect::<String>();
    let object=Object{
        binding:Binding{scope:lease.scope(),document:ResourceId::try_from(source.document_id).map_err(|_|RunFailure::Permanent)?,revision:RevisionId::try_from(source.revision_id).map_err(|_|RunFailure::Permanent)?},
        generation:Version::try_from(source.generation).map_err(|_|RunFailure::Permanent)?,
        size:u64::try_from(source.byte_count).map_err(|_|RunFailure::Permanent)?,
        mime:DeclaredMime::parse(&source.mime).map_err(|_|RunFailure::Permanent)?,
        digest:Sha256Digest::parse(&hash).map_err(|_|RunFailure::Permanent)?,
    };
    storage.read_verified(&object,Phase::Quarantine,version,chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now()))
        .await.map_err(|error|match error{
            finance_documents::s3::Error::Integrity|finance_documents::s3::Error::InvalidVersion|finance_documents::s3::Error::Metadata=>RunFailure::Permanent,
            _=>RunFailure::Transient,
        })
}

async fn recognize_document(config:&Config,lease:Lease,source:&Source)->Result<(Vec<PageResult>,Vec<Rotation>,[u8;32]),RunFailure>{
    let content=input_bytes(config,lease,source).await?;
    let mut hash=Sha256::new();
    hash.update(b"finance.local.ocr/v2/tesseract/vie/psm6");
    hash.update(config.binary_digest);
    let (pages,rotations)=match source.mime.as_str(){
        "image/png"=>{
            let (width,height)=png_dimensions(&content)?;
            let tsv=recognize(config,&content).await?;
            let page=parse_page(&tsv,1,1,width,height).map_err(|_|RunFailure::Permanent)?;
            hash.update(width.to_be_bytes());hash.update(height.to_be_bytes());
            (vec![page],vec![Rotation::R0])
        },
        "application/pdf"=>{
            let pdf=config.pdf.as_ref().ok_or(RunFailure::Permanent)?;
            hash.update(pdf.fingerprint());
            let document=pdf.open(&content).await?;
            let count=document.count();
            let mut pages=Vec::with_capacity(count as usize);
            let mut rotations=Vec::with_capacity(count as usize);
            let mut blocks=0usize;
            let mut text_bytes=0usize;
            for page in 1..=count {
                let (raster,rotation)=document.render(page).await?;
                let (width,height)=png_dimensions(&raster)?;
                let tsv=recognize(config,&raster).await?;
                let result=parse_page(&tsv,page,count,width,height).map_err(|_|RunFailure::Permanent)?;
                blocks+=result.blocks().len();
                text_bytes+=result.blocks().iter().map(|b|b.text().len()).sum::<usize>();
                if blocks>100_000 || text_bytes>8*1024*1024 {return Err(RunFailure::Permanent);}
                pages.push(result);rotations.push(rotation);
                hash.update(width.to_be_bytes());hash.update(height.to_be_bytes());
            }
            (pages,rotations)
        },
        _=>return Err(RunFailure::Permanent),
    };
    Ok((pages,rotations,hash.finalize().into()))
}

#[cfg(test)]
fn candidates(tsv:&str,width:u32,height:u32)->Result<(Value,Value),RunFailure>{
    let page=parse_page(tsv,1,1,width,height).map_err(|_|RunFailure::Permanent)?;
    let (blocks,fields,_)=candidates_for(Kind::Invoice,&[page],&[Rotation::R0])?;
    Ok((blocks,fields))
}

pub(super) async fn process(pool: &PgPool, config: &Config, lease: Lease, lease_seconds: i32) -> Result<(), RunFailure> {
    let source = source(pool, lease).await?;
    let kind=parse_kind(&source.kind)?;
    let mut work=Box::pin(time::timeout(Duration::from_secs(600),recognize_document(config,lease,&source)));
    let mut heartbeat=time::interval(Duration::from_secs((lease_seconds/3).max(1) as u64));
    heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let (pages,rotations,config_digest)=loop {
        tokio::select! {
            result=&mut work=>break result.map_err(|_|RunFailure::Transient)??,
            _=heartbeat.tick()=>jobs::heartbeat(pool,lease,lease_seconds).await.map_err(job_failure)?,
        }
    };
    let page_count=i32::try_from(pages.len()).map_err(|_|RunFailure::Permanent)?;
    let (blocks,fields,classification)=candidates_for(kind,&pages,&rotations)?;
    let scope = lease.scope();
    let (t,c): (Uuid,Uuid) = (scope.tenant.into(),scope.company.into());
    let mut tx = pool.begin().await.map_err(db_failure)?;
    sqlx::query("SELECT set_config('finance.tenant_id',$1,true),set_config('finance.company_id',$2,true)")
        .bind(t.to_string()).bind(c.to_string()).execute(&mut *tx).await.map_err(db_failure)?;
    sqlx::query_scalar::<_,Uuid>("SELECT finance.complete_local_ocr($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)")
        .bind(t).bind(c).bind(lease.job_id).bind(lease.fence).bind(lease.worker_id())
        .bind(source.document_id).bind(source.revision_id).bind(source.generation)
        .bind(&source.source_digest).bind(config.model_digest.as_slice()).bind(config_digest.as_slice())
        .bind(blocks).bind(fields).bind(classification).bind(page_count).bind(&source.kind).bind(&source.event_type).fetch_one(&mut *tx).await.map_err(db_failure)?;
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
