//! E18 persisted lifecycle commands. No generic status mutation.
use crate::draft::Draft;
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
    ids::{CompanyScope, ResourceScope},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct Ready {
    pub dossier: Uuid,
    pub check_run: Uuid,
    pub snapshot: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct Snapshot {
    pub id: Uuid,
    pub dossier_id: Uuid,
    pub revision: i64,
    pub input_hash: String,
    pub policy_version_id: Uuid,
    pub check_run_id: Uuid,
    pub sealed_at: String,
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

fn hash(request: Ready, version: i64) -> Vec<u8> {
    let mut digest = Sha256::new();
    digest.update(b"finance.dossier.ready/v1");
    digest.update(request.dossier.as_bytes());
    digest.update(request.check_run.as_bytes());
    digest.update(version.to_be_bytes());
    digest.finalize().to_vec()
}

async fn current(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant: Uuid,
    company: Uuid,
    actor: Uuid,
    dossier: Uuid,
) -> Result<Draft, Error> {
    Ok(sqlx::query_as::<_, Draft>(
        "SELECT id,company_id,code,kind,status,revision,version,owner_id,to_char(updated_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at FROM finance.dossier WHERE tenant_id=$1 AND company_id=$2 AND owner_id=$3 AND id=$4",
    )
    .bind(tenant)
    .bind(company)
    .bind(actor)
    .bind(dossier)
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn ready(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    request: Ready,
) -> Result<Draft, Error> {
    let Some(expected) = proof.context().expected_version else {
        return Err(Error::InvalidInput);
    };
    let version = i64::from(expected);
    if request.dossier.is_nil() || request.check_run.is_nil() || request.snapshot.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, actor) = binding(proof)?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let request_hash = hash(request, version);
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
            "{tenant}/{company}/{actor}/dossier.ready/{}",
            key.as_str()
        ))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='dossier.ready' AND key=$4",
    )
    .bind(tenant)
    .bind(company)
    .bind(actor)
    .bind(key.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((old, result)) = prior {
        if old != request_hash || result != request.dossier {
            return Err(Error::Conflict);
        }
        let receipt = current(&mut tx, tenant, company, actor, request.dossier).await?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(receipt);
    }
    let result: Uuid = sqlx::query_scalar(
        "SELECT finance.workflow_ready($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(tenant)
    .bind(company)
    .bind(request.dossier)
    .bind(version)
    .bind(actor)
    .bind(request.check_run)
    .bind(request.snapshot)
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| match &error {
        sqlx::Error::Database(database) if database.code().as_deref() == Some("23514") => {
            Error::Conflict
        }
        _ => Error::Database(error),
    })?;
    if result != request.snapshot {
        return Err(Error::Conflict);
    }
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'dossier.ready',$4,$5,$6)")
        .bind(tenant).bind(company).bind(actor).bind(key.as_str()).bind(request_hash).bind(request.dossier)
        .execute(&mut *tx).await?;
    let receipt = current(&mut tx, tenant, company, actor, request.dossier).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(receipt)
}

pub async fn snapshot(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
) -> Result<Option<Snapshot>, Error> {
    if dossier.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|error| match error {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(error) => Error::Database(error),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = sqlx::query_as::<_, Snapshot>(
        "SELECT s.id,s.dossier_id,r.number AS revision,encode(s.input_hash,'hex') AS input_hash,s.policy_id AS policy_version_id,s.check_run_id,to_char(s.sealed_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS sealed_at FROM finance.dossier_snapshot s JOIN finance.dossier_revision r ON (r.tenant_id,r.company_id,r.id)=(s.tenant_id,s.company_id,s.revision_id) WHERE s.tenant_id=$1 AND s.company_id=$2 AND s.dossier_id=$3 ORDER BY s.sealed_at DESC,s.id DESC LIMIT 1",
    )
    .bind(tenant)
    .bind(company)
    .bind(dossier)
    .fetch_optional(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
