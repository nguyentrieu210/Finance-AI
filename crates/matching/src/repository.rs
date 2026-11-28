//! E10 authenticated manual match confirmation and current-revision reads.
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct MatchResult {
    pub id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left_evidence_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_evidence_id: Option<Uuid>,
    pub state: String,
    pub reason: String,
}
#[derive(sqlx::FromRow)]
struct Receipt {
    id: Uuid,
    left_evidence_id: Uuid,
    right_evidence_id: Uuid,
    state: String,
    reason: String,
    dossier_version: i64,
}
pub struct MatchReceipt {
    pub result: MatchResult,
    pub dossier_version: i64,
}

pub struct ConfirmCommand<'a> {
    pub dossier: Uuid,
    pub left: Uuid,
    pub right: Uuid,
    pub reason: &'a str,
    pub result_id: Uuid,
    pub decision_id: Uuid,
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
fn grant(error: GrantError) -> Error {
    match error {
        GrantError::Forbidden => Error::Forbidden,
        GrantError::Database(error) => Error::Database(error),
    }
}
fn map_database(error: sqlx::Error) -> Error {
    match error
        .as_database_error()
        .and_then(|value| value.code())
        .as_deref()
    {
        Some("42501") => Error::Forbidden,
        Some("23514") | Some("23505") => Error::Conflict,
        _ => Error::Database(error),
    }
}
fn request_hash(dossier: Uuid, version: i64, left: Uuid, right: Uuid, reason: &str) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"finance.match.confirm/v1");
    hash.update(dossier.as_bytes());
    hash.update(version.to_be_bytes());
    hash.update(left.as_bytes());
    hash.update(right.as_bytes());
    hash.update((reason.len() as u64).to_be_bytes());
    hash.update(reason.as_bytes());
    hash.finalize().to_vec()
}
fn from_receipt(row: Receipt) -> MatchReceipt {
    MatchReceipt {
        result: MatchResult {
            id: row.id,
            left_evidence_id: Some(row.left_evidence_id),
            right_evidence_id: Some(row.right_evidence_id),
            state: row.state,
            reason: row.reason,
        },
        dossier_version: row.dossier_version,
    }
}

pub async fn list(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    after: Uuid,
    limit: i64,
) -> Result<Vec<MatchResult>, Error> {
    if dossier.is_nil() || !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, actor) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    let roles = finance_permissions::lock_roles(&mut tx, proof.context())
        .await
        .map_err(grant)?;
    if !roles.contains(&Role::Requester) && !roles.contains(&Role::Accountant) {
        return Err(Error::Forbidden);
    }
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows = sqlx::query_as::<_, MatchResult>(
        "SELECT * FROM finance.runtime_matches($1,$2,$3,$4,$5,$6::integer)",
    )
    .bind(tenant)
    .bind(company)
    .bind(dossier)
    .bind(actor)
    .bind(after)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await
    .map_err(map_database)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn confirm(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    command: ConfirmCommand<'_>,
) -> Result<MatchReceipt, Error> {
    let ConfirmCommand {
        dossier,
        left,
        right,
        reason,
        result_id,
        decision_id,
    } = command;
    let Some(expected) = proof.context().expected_version else {
        return Err(Error::InvalidInput);
    };
    let version = i64::from(expected);
    if dossier.is_nil()
        || left.is_nil()
        || right.is_nil()
        || left == right
        || result_id.is_nil()
        || decision_id.is_nil()
        || reason.trim().is_empty()
        || reason.len() > 2048
        || reason.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput);
    }
    let digest = request_hash(dossier, version, left, right, reason);
    let (tenant, company, actor) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Accountant)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "{tenant}/{company}/{actor}/match.confirm/{}",
            key.as_str()
        ))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='match.confirm' AND key=$4",
    )
    .bind(tenant)
    .bind(company)
    .bind(actor)
    .bind(key.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((old, prior_result)) = prior {
        if old != digest {
            return Err(Error::Conflict);
        }
        let row = sqlx::query_as::<_, Receipt>(
            "SELECT * FROM finance.runtime_match_receipt($1,$2,$3,$4,$5)",
        )
        .bind(tenant)
        .bind(company)
        .bind(dossier)
        .bind(actor)
        .bind(prior_result)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database)?
        .ok_or(Error::Conflict)?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(from_receipt(row));
    }

    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let new_version = sqlx::query_scalar::<_, i64>(
        "SELECT finance.runtime_match_confirm($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    )
    .bind(tenant)
    .bind(company)
    .bind(dossier)
    .bind(version)
    .bind(actor)
    .bind(result_id)
    .bind(decision_id)
    .bind(left)
    .bind(right)
    .bind(reason)
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_database)?;
    sqlx::query(
        "INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'match.confirm',$4,$5,$6)",
    )
    .bind(tenant)
    .bind(company)
    .bind(actor)
    .bind(key.as_str())
    .bind(digest)
    .bind(result_id)
    .execute(&mut *tx)
    .await?;
    let row =
        sqlx::query_as::<_, Receipt>("SELECT * FROM finance.runtime_match_receipt($1,$2,$3,$4,$5)")
            .bind(tenant)
            .bind(company)
            .bind(dossier)
            .bind(actor)
            .bind(result_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_database)?;
    if row.dossier_version != new_version {
        return Err(Error::Conflict);
    }
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(from_receipt(row))
}

pub async fn reconcile_lines(pool:&PgPool,proof:&AuthenticatedCommand,key:&IdempotencyKey,dossier:Uuid,policy:Uuid,material:&serde_json::Value,id:Uuid)->Result<serde_json::Value,Error>{
 let version=i64::from(proof.context().expected_version.ok_or(Error::InvalidInput)?);
 let(t,c,a)=binding(proof)?;
 let mut tx=scoped_transaction(pool,proof.context()).await?;
 require_role(&mut tx,proof.context(),Role::Accountant).await.map_err(grant)?;
 recheck_authenticated(&mut tx,Some(proof)).await?;
 let cmd:Uuid=proof.context().command.into();let corr:Uuid=proof.context().correlation.into();
 let result=sqlx::query_scalar::<_,serde_json::Value>("SELECT finance.runtime_line_reconcile($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
 .bind(t).bind(c).bind(a).bind(dossier).bind(version).bind(policy).bind(material).bind(key.as_str()).bind(id).bind(cmd).bind(corr).fetch_one(&mut *tx).await.map_err(map_database)?;
 recheck_authenticated(&mut tx,Some(proof)).await?;tx.commit().await?;Ok(result)
}
pub async fn line_history(pool:&PgPool,proof:&AuthenticatedCommand,dossier:Uuid)->Result<serde_json::Value,Error>{
 let(t,c,a)=binding(proof)?;let mut tx=scoped_transaction(pool,proof.context()).await?;
 finance_permissions::lock_roles(&mut tx,proof.context()).await.map_err(grant)?;
 recheck_authenticated(&mut tx,Some(proof)).await?;
 let result=sqlx::query_scalar::<_,serde_json::Value>("SELECT finance.runtime_line_reconciliations($1,$2,$3,$4)").bind(t).bind(c).bind(a).bind(dossier).fetch_one(&mut *tx).await.map_err(map_database)?;
 recheck_authenticated(&mut tx,Some(proof)).await?;tx.commit().await?;Ok(result)
}
