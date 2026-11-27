//! E20 immutable template registry. Source digest is derived from a CLEAN blob.
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, lock_roles, require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
    ids::{CompanyScope, ResourceScope},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct Template {
    pub id: Uuid,
    pub policy_id: Uuid,
    pub version: i64,
    pub name: String,
    pub format: String,
    pub sha256: String,
    pub state: String,
}

pub struct CreateDraft<'a> {
    pub policy: Uuid,
    pub template: Uuid,
    pub row: Uuid,
    pub name: &'a str,
    pub document: Uuid,
    pub format: &'a str,
}
pub struct ReviseDraft<'a> {
    pub template: Uuid,
    pub row: Uuid,
    pub document: Uuid,
    pub reason: &'a str,
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
fn valid_format(format: &str) -> bool {
    matches!(format, "DOCX" | "XLSX" | "PDF")
}
fn request_hash(parts: &[&[u8]]) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"finance.template.command/v1");
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.finalize().to_vec()
}
const SELECT: &str = "SELECT template_id AS id,policy_id,version,name,format,encode(source_digest,'hex') AS sha256,CASE WHEN state='REVOKED' THEN 'RETIRED' ELSE state END AS state FROM finance.template_version";

async fn allowed_read(
    tx: &mut Transaction<'_, Postgres>,
    proof: &AuthenticatedCommand,
) -> Result<(), Error> {
    let roles = lock_roles(tx, proof.context())
        .await
        .map_err(|error| match error {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(error) => Error::Database(error),
        })?;
    if roles.contains(&Role::Requester) || roles.contains(&Role::PolicyAdmin) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}
async fn latest(
    tx: &mut Transaction<'_, Postgres>,
    tenant: Uuid,
    company: Uuid,
    template: Uuid,
) -> Result<Template, Error> {
    Ok(sqlx::query_as::<_, Template>(&format!(
        "{SELECT} WHERE tenant_id=$1 AND company_id=$2 AND template_id=$3 ORDER BY version DESC LIMIT 1"
    ))
    .bind(tenant)
    .bind(company)
    .bind(template)
    .fetch_one(&mut **tx)
    .await?)
}
async fn source_digest(
    tx: &mut Transaction<'_, Postgres>,
    document: Uuid,
    format: &str,
) -> Result<String, Error> {
    Ok(sqlx::query_scalar("SELECT encode(finance.template_source_digest($1,$2),'hex')")
        .bind(document)
        .bind(format)
        .fetch_one(&mut **tx)
        .await?)
}

pub async fn create(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    draft: CreateDraft<'_>,
) -> Result<Template, Error> {
    if draft.policy.is_nil()
        || draft.template.is_nil()
        || draft.row.is_nil()
        || draft.document.is_nil()
        || !valid_format(draft.format)
        || draft.name.trim().is_empty()
        || draft.name.len() > 200
        || proof.context().expected_version.is_some()
    {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, actor) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::PolicyAdmin)
        .await
        .map_err(|error| match error {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(error) => Error::Database(error),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let digest = source_digest(&mut tx, draft.document, draft.format).await?;
    let request = request_hash(&[
        draft.policy.as_bytes(),
        draft.name.as_bytes(),
        draft.document.as_bytes(),
        draft.format.as_bytes(),
        digest.as_bytes(),
    ]);
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "{tenant}/{company}/{actor}/template.create/{}",
            key.as_str()
        ))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='template.create' AND key=$4",
    )
    .bind(tenant).bind(company).bind(actor).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((old, result)) = prior {
        if old != request {
            return Err(Error::Conflict);
        }
        let receipt = latest(&mut tx, tenant, company, result).await?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(receipt);
    }
    let policy_ok: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM finance.policy_version WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND author_id=$4 AND state='DRAFT')",
    )
    .bind(tenant).bind(company).bind(draft.policy).bind(actor).fetch_one(&mut *tx).await?;
    if !policy_ok {
        return Err(Error::Conflict);
    }
    sqlx::query(
        "INSERT INTO finance.template_version(tenant_id,company_id,id,template_id,policy_id,name,version,state,format,source_digest,document_id,author_id) VALUES($1,$2,$3,$4,$5,$6,1,'DRAFT',$7,decode($8,'hex'),$9,$10)",
    )
    .bind(tenant).bind(company).bind(draft.row).bind(draft.template).bind(draft.policy)
    .bind(draft.name).bind(draft.format).bind(&digest).bind(draft.document).bind(actor)
    .execute(&mut *tx).await?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'TEMPLATE_DRAFT_CREATED')")
        .bind(tenant).bind(company).bind(command).bind(actor).bind(correlation).bind(draft.template)
        .execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'TEMPLATE_DRAFT_CREATED')")
        .bind(tenant).bind(company).bind(command).bind(draft.template).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'template.create',$4,$5,$6)")
        .bind(tenant).bind(company).bind(actor).bind(key.as_str()).bind(request).bind(draft.template)
        .execute(&mut *tx).await?;
    let receipt = latest(&mut tx, tenant, company, draft.template).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(receipt)
}

pub async fn revise(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    draft: ReviseDraft<'_>,
) -> Result<Template, Error> {
    let Some(expected) = proof.context().expected_version else {
        return Err(Error::InvalidInput);
    };
    let expected = i64::from(expected);
    if draft.template.is_nil()
        || draft.row.is_nil()
        || draft.document.is_nil()
        || draft.reason.trim().is_empty()
        || draft.reason.len() > 2048
    {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, actor) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::PolicyAdmin)
        .await
        .map_err(|error| match error {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(error) => Error::Database(error),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "{tenant}/{company}/{actor}/template.revise/{}",
            key.as_str()
        ))
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "finance.template/v1/{tenant}/{company}/{}",
            draft.template
        ))
        .execute(&mut *tx)
        .await?;
    let current = latest(&mut tx, tenant, company, draft.template).await?;
    let digest = source_digest(&mut tx, draft.document, &current.format).await?;
    let request = request_hash(&[
        draft.template.as_bytes(),
        &expected.to_be_bytes(),
        draft.document.as_bytes(),
        digest.as_bytes(),
        draft.reason.as_bytes(),
    ]);
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='template.revise' AND key=$4",
    )
    .bind(tenant).bind(company).bind(actor).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((old, result)) = prior {
        if old != request || result != draft.template {
            return Err(Error::Conflict);
        }
        let receipt = latest(&mut tx, tenant, company, draft.template).await?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(receipt);
    }
    if current.version != expected || current.state != "DRAFT" {
        return Err(Error::Conflict);
    }
    let next = expected.checked_add(1).ok_or(Error::Conflict)?;
    sqlx::query(
        "INSERT INTO finance.template_version(tenant_id,company_id,id,template_id,policy_id,name,version,state,format,source_digest,document_id,author_id) SELECT tenant_id,company_id,$4,template_id,policy_id,name,$5,'DRAFT',format,decode($6,'hex'),$7,author_id FROM finance.template_version WHERE tenant_id=$1 AND company_id=$2 AND template_id=$3 AND version=$8 AND author_id=$9",
    )
    .bind(tenant).bind(company).bind(draft.template).bind(draft.row).bind(next).bind(&digest)
    .bind(draft.document).bind(expected).bind(actor).execute(&mut *tx).await?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'TEMPLATE_DRAFT_REVISED')")
        .bind(tenant).bind(company).bind(command).bind(actor).bind(correlation).bind(draft.template)
        .execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'TEMPLATE_DRAFT_REVISED')")
        .bind(tenant).bind(company).bind(command).bind(draft.template).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'template.revise',$4,$5,$6)")
        .bind(tenant).bind(company).bind(actor).bind(key.as_str()).bind(request).bind(draft.template)
        .execute(&mut *tx).await?;
    let receipt = latest(&mut tx, tenant, company, draft.template).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(receipt)
}

pub async fn list(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Template>, Error> {
    if !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    allowed_read(&mut tx, proof).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows = sqlx::query_as::<_, Template>(
        "SELECT id,policy_id,version,name,format,sha256,state FROM (SELECT DISTINCT ON(template_id) template_id AS id,policy_id,version,name,format,encode(source_digest,'hex') AS sha256,CASE WHEN state='REVOKED' THEN 'RETIRED' ELSE state END AS state FROM finance.template_version WHERE tenant_id=$1 AND company_id=$2 AND template_id>$3 ORDER BY template_id,version DESC) x ORDER BY id LIMIT $4",
    )
    .bind(tenant).bind(company).bind(after).bind(limit).fetch_all(&mut *tx).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn get(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    id: Uuid,
) -> Result<Option<Template>, Error> {
    if id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    allowed_read(&mut tx, proof).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = sqlx::query_as::<_, Template>(&format!(
        "{SELECT} WHERE tenant_id=$1 AND company_id=$2 AND template_id=$3 ORDER BY version DESC LIMIT 1"
    ))
    .bind(tenant).bind(company).bind(id).fetch_optional(&mut *tx).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
