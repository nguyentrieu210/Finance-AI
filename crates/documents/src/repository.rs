//! Synthetic owner-bound metadata commands. No file bytes or storage grants.
use super::check_name;
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
    ids::ResourceScope,
    upload_contract::{
        DEFAULT_MAX_DOSSIER_BYTES, DEFAULT_MAX_FILE_BYTES, DeclaredMime, Sha256Digest,
        UploadMetadata,
    },
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

pub struct Metadata<'a> {
    pub filename: &'a str,
    pub mime: &'a str,
    pub bytes: u64,
    pub sha256: &'a str,
    pub kind: &'a str,
}

#[derive(Serialize)]
pub struct Receipt {
    pub document_id: Uuid,
    pub required_sha256: String,
    pub state: &'static str,
    #[serde(skip)]
    pub version: i64,
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Document {
    pub id: Uuid,
    pub dossier_id: Uuid,
    pub kind: String,
    pub file_name: String,
    pub sha256: String,
    pub state: String,
    pub revision: i64,
}

fn binding(proof: &AuthenticatedCommand) -> Result<(Uuid, Uuid, Uuid), Error> {
    let ResourceScope::Company(scope) = proof.context().scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(actor) = proof.context().actor else {
        return Err(Error::Forbidden);
    };
    Ok((scope.tenant.into(), scope.company.into(), actor.into()))
}

fn grant_error(error: GrantError) -> Error {
    match error {
        GrantError::Forbidden => Error::Forbidden,
        GrantError::Database(error) => Error::Database(error),
    }
}

fn request_hash(dossier: Uuid, version: i64, input: &Metadata<'_>) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(b"finance.createDocument/v1");
    h.update(dossier.as_bytes());
    h.update(version.to_be_bytes());
    h.update(input.bytes.to_be_bytes());
    for value in [input.filename, input.mime, input.sha256, input.kind] {
        h.update((value.len() as u64).to_be_bytes());
        h.update(value.as_bytes());
    }
    h.finalize().to_vec()
}

pub async fn create(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    dossier: Uuid,
    document: Uuid,
    input: &Metadata<'_>,
) -> Result<Receipt, Error> {
    if dossier.is_nil()
        || document.is_nil()
        || check_name(input.filename).is_err()
        || input.kind.is_empty()
        || input.kind.len() > 64
        || !input.kind.as_bytes()[0].is_ascii_uppercase()
        || !input
            .kind
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(Error::InvalidInput);
    }
    let mime = DeclaredMime::parse(input.mime).map_err(|_| Error::InvalidInput)?;
    let digest = Sha256Digest::parse(input.sha256).map_err(|_| Error::InvalidInput)?;
    let metadata = UploadMetadata::new(input.bytes, digest, mime, DEFAULT_MAX_FILE_BYTES)
        .map_err(|_| Error::InvalidInput)?;
    let expected = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(Error::InvalidInput)?,
    );
    let (t, c, a) = binding(proof)?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    let lock = format!("{t}/{c}/{a}/prototype.createDocument/{}", key.as_str());
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(lock)
        .execute(&mut *tx)
        .await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(grant_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let (revision, version): (Uuid, i64) = sqlx::query_as("SELECT r.id,d.version FROM finance.dossier d JOIN finance.dossier_revision r ON (r.tenant_id,r.company_id,r.dossier_id,r.number)=(d.tenant_id,d.company_id,d.id,d.revision) WHERE d.tenant_id=$1 AND d.company_id=$2 AND d.owner_id=$3 AND d.id=$4 AND d.status='DRAFT' FOR UPDATE OF d")
        .bind(t).bind(c).bind(a).bind(dossier).fetch_optional(&mut *tx).await?.ok_or(Error::Forbidden)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let hash = request_hash(dossier, expected, input);
    let prior: Option<(Vec<u8>,Uuid,i64)> = sqlx::query_as("SELECT request_hash,result_id,result_version FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='prototype.createDocument' AND key=$4")
        .bind(t).bind(c).bind(a).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((old, id, receipt_version)) = prior {
        if old != hash {
            return Err(Error::Conflict);
        }
        let digest: String = sqlx::query_scalar("SELECT encode(declared_sha256,'hex') FROM finance.upload_intent WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND dossier_id=$4 AND actor_id=$5")
            .bind(t).bind(c).bind(id).bind(dossier).bind(a).fetch_one(&mut *tx).await?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(Receipt {
            document_id: id,
            required_sha256: digest,
            state: "PENDING_UPLOAD",
            version: receipt_version,
        });
    }
    if expected != version || version >= 9_007_199_254_740_991 {
        return Err(Error::Conflict);
    }
    let quota_lock = format!("finance.upload.quota/v1/{t}/{c}/{dossier}");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(quota_lock)
        .execute(&mut *tx)
        .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let used: i64 = sqlx::query_scalar("SELECT COALESCE(sum(declared_bytes),0)::bigint FROM finance.upload_intent WHERE tenant_id=$1 AND company_id=$2 AND dossier_id=$3 AND state IN ('PENDING_UPLOAD','FINALIZED')")
        .bind(t).bind(c).bind(dossier).fetch_one(&mut *tx).await?;
    metadata
        .check_quota(
            u64::try_from(used).map_err(|_| Error::Conflict)?,
            DEFAULT_MAX_DOSSIER_BYTES,
        )
        .map_err(|_| Error::Conflict)?;
    sqlx::query("INSERT INTO finance.upload_intent(tenant_id,company_id,id,dossier_id,revision_id,actor_id,filename,declared_bytes,declared_sha256,declared_mime,document_kind) VALUES($1,$2,$3,$4,$5,$6,$7,$8,decode($9,'hex'),$10,$11)")
        .bind(t).bind(c).bind(document).bind(dossier).bind(revision).bind(a).bind(input.filename).bind(input.bytes as i64).bind(input.sha256).bind(input.mime).bind(input.kind).execute(&mut *tx).await?;
    let version: i64 = sqlx::query_scalar("UPDATE finance.dossier SET version=version+1,updated_at=GREATEST(updated_at,clock_timestamp()) WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND owner_id=$4 AND version=$5 RETURNING version")
        .bind(t).bind(c).bind(dossier).bind(a).bind(expected).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'DOCUMENT_INTENT_CREATED')")
        .bind(t).bind(c).bind(command).bind(a).bind(correlation).bind(document).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'DOCUMENT_INTENT_CREATED')")
        .bind(t).bind(c).bind(command).bind(document).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_version) VALUES($1,$2,$3,'prototype.createDocument',$4,$5,$6,$7)")
        .bind(t).bind(c).bind(a).bind(key.as_str()).bind(hash).bind(document).bind(version).execute(&mut *tx).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(Receipt {
        document_id: document,
        required_sha256: input.sha256.into(),
        state: "PENDING_UPLOAD",
        version,
    })
}

pub async fn list(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Document>, Error> {
    if !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(grant_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let owned: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM finance.dossier WHERE tenant_id=$1 AND company_id=$2 AND owner_id=$3 AND id=$4)")
        .bind(t).bind(c).bind(a).bind(dossier).fetch_one(&mut *tx).await?;
    if !owned {
        return Err(Error::Forbidden);
    }
    let rows = sqlx::query_as::<_,Document>("SELECT u.id,u.dossier_id,u.document_kind AS kind,u.filename AS file_name,encode(u.declared_sha256,'hex') AS sha256,finance.runtime_document_state(u.tenant_id,u.company_id,u.actor_id,u.id) AS state,r.number AS revision FROM finance.upload_intent u JOIN finance.dossier_revision r ON (r.tenant_id,r.company_id,r.id)=(u.tenant_id,u.company_id,u.revision_id) WHERE u.tenant_id=$1 AND u.company_id=$2 AND u.actor_id=$3 AND u.dossier_id=$4 AND u.id>$5 AND u.state<>'CANCELLED' AND u.document_kind IS NOT NULL ORDER BY u.id LIMIT $6")
        .bind(t).bind(c).bind(a).bind(dossier).bind(after).bind(limit).fetch_all(&mut *tx).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Evidence {
    pub id: Uuid,
    pub document_id: Uuid,
    pub sha256: String,
    pub page: i32,
    pub locator: String,
    pub excerpt: String,
}

/// Current-revision evidence only. The SQL projection enforces dossier ownership
/// or an independent accountant assignment; direct table access is not granted.
pub async fn get_evidence(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    id: Uuid,
) -> Result<Option<Evidence>, Error> {
    if id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = sqlx::query_as::<_, Evidence>("SELECT * FROM finance.runtime_evidence($1,$2,$3,$4)")
        .bind(t)
        .bind(c)
        .bind(a)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

#[derive(sqlx::FromRow)]
pub struct Original {
    pub sha256: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

pub async fn original_digest(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    document: Uuid,
) -> Result<Option<String>, Error> {
    if dossier.is_nil() || document.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let digest = sqlx::query_scalar("SELECT finance.runtime_original_digest($1,$2,$3,$4,$5)")
        .bind(t)
        .bind(c)
        .bind(a)
        .bind(dossier)
        .bind(document)
        .fetch_one(&mut *tx)
        .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(digest)
}

pub async fn download_original(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    document: Uuid,
) -> Result<Option<Original>, Error> {
    download_original_with_storage(pool,proof,dossier,document,None).await
}

pub async fn download_original_with_storage(
    pool: &PgPool, proof: &AuthenticatedCommand, dossier: Uuid, document: Uuid,
    storage: Option<&super::s3::S3>,
) -> Result<Option<Original>, Error> {
    if dossier.is_nil() || document.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let mut original = sqlx::query_as::<_, Original>(
        "SELECT * FROM finance.runtime_download_original($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(dossier)
    .bind(document)
    .bind(command)
    .bind(correlation)
    .fetch_optional(&mut *tx)
    .await?;
    if original.is_none() {
        if let Some(storage) = storage {
            original = super::storage_runtime::download(&mut tx,storage,t,c,a,dossier,document,command,correlation).await?;
        }
    }
    if let Some(row) = &original {
        if row.bytes.is_empty()
            || row.bytes.len() > DEFAULT_MAX_FILE_BYTES as usize
            || format!("{:x}", Sha256::digest(&row.bytes)) != row.sha256
        {
            return Err(Error::Conflict);
        }
    }
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(original)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct UploadGrant {
    pub document_id: Uuid,
    pub generation: i64,
    pub upload_url: String,
    pub expires_at: String,
    pub content_type: String,
    pub checksum_sha256_base64: String,
    #[serde(skip)]
    pub dossier_version: i64,
}

#[derive(Debug)]
pub enum UploadGrantError {
    Inactive,
    Core(Error),
}

impl From<Error> for UploadGrantError {
    fn from(value: Error) -> Self {
        Self::Core(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct PipelineJob {
    pub id: Uuid,
    pub state: String,
    pub attempt: i64,
    pub progress: i64,
    #[serde(skip)]
    pub dossier_version: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct SupersededDossier {
    pub id: Uuid,
    pub company_id: Uuid,
    pub code: String,
    pub kind: String,
    pub status: String,
    pub revision: i64,
    pub version: i64,
    pub owner_id: Uuid,
    pub updated_at: String,
}

fn map_pg_error(error: sqlx::Error) -> Error {
    match &error {
        sqlx::Error::Database(db) => match db.code().as_deref() {
            Some("22023") => Error::InvalidInput,
            Some("42501") => Error::Forbidden,
            Some("23505") => Error::Conflict,
            _ => Error::Database(error),
        },
        _ => Error::Database(error),
    }
}

pub async fn issue_upload_grant(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    dossier: Uuid,
    document: Uuid,
) -> Result<UploadGrant, UploadGrantError> {
    issue_upload_grant_with_storage(pool, proof, key, dossier, document, None).await
}

pub async fn issue_upload_grant_with_storage(
    pool: &PgPool, proof: &AuthenticatedCommand, key: &IdempotencyKey,
    dossier: Uuid, document: Uuid, storage: Option<&super::s3::S3>,
) -> Result<UploadGrant, UploadGrantError> {
    if dossier.is_nil() || document.is_nil() {
        return Err(UploadGrantError::Core(Error::InvalidInput));
    }
    let expected = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(UploadGrantError::Core(Error::InvalidInput))?,
    );
    let (t, c, a) = binding(proof)?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let mut tx = scoped_transaction(pool, proof.context())
        .await
        .map_err(UploadGrantError::from)?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|e| UploadGrantError::Core(grant_error(e)))?;
    recheck_authenticated(&mut tx, Some(proof))
        .await
        .map_err(UploadGrantError::from)?;
    let result = sqlx::query_as::<_, UploadGrant>(
        "SELECT * FROM finance.runtime_issue_upload_grant($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(dossier)
    .bind(document)
    .bind(expected)
    .bind(key.as_str())
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await;
    let mut grant = match result {
        Ok(row) => row,
        Err(sqlx::Error::Database(db))
            if db.code().as_deref() == Some("P0001")
                && db.message().contains("UPLOAD_GRANT_INACTIVE") =>
        {
            return Err(UploadGrantError::Inactive);
        }
        Err(err) => return Err(UploadGrantError::Core(map_pg_error(err))),
    };
    if let Some(storage) = storage {
        let metadata = super::storage_runtime::grant_metadata(&mut tx, t,c,a,dossier,document,grant.generation)
            .await.map_err(UploadGrantError::from)?;
        let object = metadata.object(t,c,document).map_err(UploadGrantError::from)?;
        let expires = chrono::DateTime::parse_from_rfc3339(&grant.expires_at)
            .map_err(|_| UploadGrantError::Core(Error::Conflict))?.with_timezone(&chrono::Utc);
        let issued = expires - chrono::TimeDelta::seconds(300);
        let signed = storage.upload_grant(&object,issued,300)
            .map_err(|_| UploadGrantError::Core(Error::Conflict))?;
        grant.upload_url = signed.url.to_string();
    }
    recheck_authenticated(&mut tx, Some(proof))
        .await
        .map_err(UploadGrantError::from)?;
    tx.commit()
        .await
        .map_err(|e| UploadGrantError::Core(Error::Database(e)))?;
    Ok(grant)
}

#[derive(Clone, Copy, Debug)]
pub struct FinalizeInput<'a> {
    pub dossier: Uuid,
    pub document: Uuid,
    pub sha256: &'a str,
    pub generation: i64,
    pub job_id: Uuid,
}

pub async fn finalize(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    input: FinalizeInput<'_>,
) -> Result<PipelineJob, Error> {
    finalize_with_storage(pool,proof,key,input,None).await
}

pub async fn finalize_with_storage(
    pool: &PgPool, proof: &AuthenticatedCommand, key: &IdempotencyKey,
    input: FinalizeInput<'_>, storage: Option<&super::s3::S3>,
) -> Result<PipelineJob, Error> {
    if input.dossier.is_nil()
        || input.document.is_nil()
        || input.job_id.is_nil()
        || input.generation < 1
        || Sha256Digest::parse(input.sha256).is_err()
    {
        return Err(Error::InvalidInput);
    }
    let expected = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(Error::InvalidInput)?,
    );
    let (t, c, a) = binding(proof)?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(grant_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = sqlx::query_as::<_, PipelineJob>(
        "SELECT * FROM finance.runtime_finalize_document($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(input.dossier)
    .bind(input.document)
    .bind(expected)
    .bind(input.sha256)
    .bind(input.generation)
    .bind(key.as_str())
    .bind(input.job_id)
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_pg_error)?;
    if let Some(storage) = storage {
        super::storage_runtime::attach_finalized(&mut tx,storage,t,c,a,input.dossier,input.document,input.generation,row.id).await?;
    }
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn supersede(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    dossier: Uuid,
    document: Uuid,
    replacement_id: Uuid,
    reason: &str,
) -> Result<SupersededDossier, Error> {
    if dossier.is_nil()
        || document.is_nil()
        || replacement_id.is_nil()
        || document == replacement_id
        || reason.trim().is_empty()
        || reason.len() > 4096
    {
        return Err(Error::InvalidInput);
    }
    let expected = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(Error::InvalidInput)?,
    );
    let (t, c, a) = binding(proof)?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(grant_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = sqlx::query_as::<_, SupersededDossier>(
        "SELECT * FROM finance.runtime_supersede_document($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(dossier)
    .bind(document)
    .bind(expected)
    .bind(replacement_id)
    .bind(reason)
    .bind(key.as_str())
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_pg_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

#[derive(Clone, Copy, Debug)]
pub struct PipelineJobInput<'a> {
    pub dossier: Uuid,
    pub document: Uuid,
    pub operation: &'a str,
    pub value: &'a str,
    pub job_id: Uuid,
}

pub async fn request_pipeline_job(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    input: PipelineJobInput<'_>,
) -> Result<PipelineJob, Error> {
    if input.dossier.is_nil()
        || input.document.is_nil()
        || input.job_id.is_nil()
        || input.value.trim().is_empty()
        || input.value.len() > 4096
        || !matches!(
            input.operation,
            "document.ocr" | "document.classify" | "document.extract"
        )
    {
        return Err(Error::InvalidInput);
    }
    let expected = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(Error::InvalidInput)?,
    );
    let (t, c, a) = binding(proof)?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(grant_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = sqlx::query_as::<_, PipelineJob>(
        "SELECT * FROM finance.runtime_document_pipeline_job($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(input.dossier)
    .bind(input.document)
    .bind(expected)
    .bind(input.operation)
    .bind(input.value)
    .bind(key.as_str())
    .bind(input.job_id)
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_pg_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn rendered_evidence(pool:&PgPool,proof:&AuthenticatedCommand,id:Uuid)->Result<Option<Original>,Error>{
 if id.is_nil(){return Err(Error::InvalidInput)}
 let (t,c,a)=binding(proof)?;
 let command:Uuid=proof.context().command.into();
 let correlation:Uuid=proof.context().correlation.into();
 let mut tx=scoped_transaction(pool,proof.context()).await?;
 recheck_authenticated(&mut tx,Some(proof)).await?;
 let row=sqlx::query_as::<_,Original>("SELECT * FROM finance.runtime_evidence_rendered($1,$2,$3,$4,$5,$6)")
  .bind(t).bind(c).bind(a).bind(id).bind(command).bind(correlation).fetch_optional(&mut *tx).await?;
 if let Some(row)=&row { if row.bytes.len()>DEFAULT_MAX_FILE_BYTES as usize||!row.bytes.starts_with(b"%PDF-")||format!("{:x}",Sha256::digest(&row.bytes))!=row.sha256{return Err(Error::Conflict)} }
 recheck_authenticated(&mut tx,Some(proof)).await?;tx.commit().await?;Ok(row)
}
