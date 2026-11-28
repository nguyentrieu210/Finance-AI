//! Database-backed S3 lifecycle. External storage is enabled only by trusted
//! process configuration; access to object bytes always requires current DB scope.
use super::{
    Binding,
    repository::Original,
    s3::{Object, Phase, S3},
};
use finance_core::prototype::Error;
use finance_shared::{
    contracts::Version,
    ids::{CompanyId, CompanyScope, ResourceId, RevisionId, TenantId},
    upload_contract::{DeclaredMime, Sha256Digest},
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub struct Metadata {
    revision_id: Uuid,
    generation: i64,
    byte_count: i64,
    mime: String,
    sha256: String,
    object_version: Option<String>,
}

impl Metadata {
    pub fn object(&self, t: Uuid, c: Uuid, document: Uuid) -> Result<Object, Error> {
        Ok(Object {
            binding: Binding {
                scope: CompanyScope {
                    tenant: TenantId::try_from(t).map_err(|_| Error::Conflict)?,
                    company: CompanyId::try_from(c).map_err(|_| Error::Conflict)?,
                },
                document: ResourceId::try_from(document).map_err(|_| Error::Conflict)?,
                revision: RevisionId::try_from(self.revision_id).map_err(|_| Error::Conflict)?,
            },
            generation: Version::try_from(self.generation).map_err(|_| Error::Conflict)?,
            size: u64::try_from(self.byte_count).map_err(|_| Error::Conflict)?,
            mime: DeclaredMime::parse(&self.mime).map_err(|_| Error::Conflict)?,
            digest: Sha256Digest::parse(&self.sha256).map_err(|_| Error::Conflict)?,
        })
    }
}

pub(super) async fn grant_metadata(
    tx: &mut Transaction<'_, Postgres>,
    t: Uuid,
    c: Uuid,
    a: Uuid,
    d: Uuid,
    doc: Uuid,
    g: i64,
) -> Result<Metadata, Error> {
    sqlx::query_as::<_, Metadata>(
        "SELECT * FROM finance.runtime_storage_metadata($1,$2,$3,$4,$5,$6)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(d)
    .bind(doc)
    .bind(g)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(Error::Conflict)
}

#[derive(Clone, Copy)]
pub(super) struct DocumentScope {
    pub tenant: Uuid,
    pub company: Uuid,
    pub actor: Uuid,
    pub dossier: Uuid,
    pub document: Uuid,
}

pub(super) async fn attach_finalized(
    tx: &mut Transaction<'_, Postgres>,
    storage: &S3,
    scope: DocumentScope,
    g: i64,
    job: Uuid,
) -> Result<(), Error> {
    let DocumentScope {
        tenant: t,
        company: c,
        actor: a,
        dossier: d,
        document: doc,
    } = scope;
    let metadata = grant_metadata(tx, t, c, a, d, doc, g).await?;
    if metadata.object_version.is_some() {
        return Ok(());
    }
    let object = metadata.object(t, c, doc)?;
    let now = chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now());
    let (staged, original) = storage
        .finalize_staged(&object, now)
        .await
        .map_err(|_| Error::Conflict)?;
    sqlx::query("SELECT finance.runtime_attach_s3_original($1,$2,$3,$4,$5,$6,$7,$8,$9)")
        .bind(t)
        .bind(c)
        .bind(a)
        .bind(d)
        .bind(doc)
        .bind(g)
        .bind(job)
        .bind(staged)
        .bind(original)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn download(
    tx: &mut Transaction<'_, Postgres>,
    storage: &S3,
    scope: DocumentScope,
    command: Uuid,
    correlation: Uuid,
) -> Result<Option<Original>, Error> {
    let DocumentScope {
        tenant: t,
        company: c,
        actor: a,
        dossier: d,
        document: doc,
    } = scope;
    let metadata = sqlx::query_as::<_, Metadata>(
        "SELECT * FROM finance.runtime_s3_download($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(d)
    .bind(doc)
    .bind(command)
    .bind(correlation)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(metadata) = metadata else {
        return Ok(None);
    };
    let object = metadata.object(t, c, doc)?;
    let version = metadata.object_version.as_deref().ok_or(Error::Conflict)?;
    let bytes = storage
        .read_verified(
            &object,
            Phase::Quarantine,
            version,
            chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now()),
        )
        .await
        .map_err(|_| Error::Conflict)?;
    Ok(Some(Original {
        sha256: metadata.sha256,
        mime: metadata.mime,
        bytes,
    }))
}
