//! E18 persisted accountant review lifecycle.
use crate::draft::Draft;
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
pub struct Review {
    pub id: Uuid,
    pub snapshot_id: Uuid,
    pub reviewer_id: Uuid,
    pub decision: String,
    pub confirmed_field_ids: Vec<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip)]
    pub dossier_version: i64,
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

fn valid_reason(reason: &str) -> bool {
    !reason.trim().is_empty() && reason.len() <= 4096 && !reason.chars().any(char::is_control)
}

fn canonical_ids(values: &[Uuid], max: usize) -> Result<Vec<Uuid>, Error> {
    if values.len() > max || values.iter().any(Uuid::is_nil) {
        return Err(Error::InvalidInput);
    }
    let mut result = values.to_vec();
    result.sort();
    result.dedup();
    if result.len() != values.len() {
        return Err(Error::InvalidInput);
    }
    Ok(result)
}

fn digest(parts: &[&[u8]]) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"finance.accounting-review/v1");
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.finalize().to_vec()
}

async fn review_by_dossier(
    tx: &mut Transaction<'_, Postgres>,
    t: Uuid,
    c: Uuid,
    d: Uuid,
    a: Uuid,
) -> Result<Option<Review>, Error> {
    Ok(sqlx::query_as::<_, Review>(
        "SELECT id,snapshot_id,reviewer_id,decision,confirmed_field_ids,reason,dossier_version          FROM finance.runtime_review_get($1,$2,$3,$4)",
    )
    .bind(t)
    .bind(c)
    .bind(d)
    .bind(a)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn accountant_dossier(
    tx: &mut Transaction<'_, Postgres>,
    t: Uuid,
    c: Uuid,
    d: Uuid,
    a: Uuid,
) -> Result<Draft, Error> {
    Ok(sqlx::query_as::<_, Draft>(
        "SELECT * FROM finance.runtime_review_dossier($1,$2,$3,$4)",
    )
    .bind(t)
    .bind(c)
    .bind(d)
    .bind(a)
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn begin(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    dossier: Uuid,
    review: Uuid,
    reason: &str,
    evidence: &[Uuid],
) -> Result<Draft, Error> {
    let Some(expected) = proof.context().expected_version else {
        return Err(Error::InvalidInput);
    };
    let expected = i64::from(expected);
    if dossier.is_nil() || review.is_nil() || !valid_reason(reason) {
        return Err(Error::InvalidInput);
    }
    let evidence = canonical_ids(evidence, 64)?;
    let expected_bytes = expected.to_be_bytes();
    let mut evidence_bytes = Vec::with_capacity(evidence.len() * 16);
    for id in &evidence {
        evidence_bytes.extend_from_slice(id.as_bytes());
    }
    let hash = digest(&[
        b"begin",
        dossier.as_bytes(),
        &expected_bytes,
        reason.as_bytes(),
        &evidence_bytes,
    ]);
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Accountant)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/review.begin/{}", key.as_str()))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record          WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='review.begin' AND key=$4",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(key.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((old, result)) = prior {
        if old != hash {
            return Err(Error::Conflict);
        }
        let stored = review_by_dossier(&mut tx, t, c, dossier, a).await?;
        if stored.as_ref().is_none_or(|row| row.id != result) {
            return Err(Error::Conflict);
        }
        let receipt = accountant_dossier(&mut tx, t, c, dossier, a).await?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(receipt);
    }
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let (result, _version): (Uuid, i64) = sqlx::query_as(
        "SELECT review_id_out,dossier_version_out FROM finance.workflow_begin_accounting_review(         $1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(t)
    .bind(c)
    .bind(dossier)
    .bind(expected)
    .bind(a)
    .bind(review)
    .bind(reason)
    .bind(&evidence)
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_guard)?;
    sqlx::query(
        "INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id)          VALUES($1,$2,$3,'review.begin',$4,$5,$6)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(key.as_str())
    .bind(hash)
    .bind(result)
    .execute(&mut *tx)
    .await?;
    let receipt = accountant_dossier(&mut tx, t, c, dossier, a).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(receipt)
}

pub async fn decide(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    dossier: Uuid,
    snapshot: Uuid,
    decision: &str,
    confirmed: &[Uuid],
    reason: &str,
) -> Result<Review, Error> {
    let Some(expected) = proof.context().expected_version else {
        return Err(Error::InvalidInput);
    };
    let expected = i64::from(expected);
    if dossier.is_nil()
        || snapshot.is_nil()
        || !matches!(decision, "ACCEPTED" | "RETURNED")
        || !valid_reason(reason)
    {
        return Err(Error::InvalidInput);
    }
    let confirmed = canonical_ids(confirmed, 256)?;
    let expected_bytes = expected.to_be_bytes();
    let mut confirmed_bytes = Vec::with_capacity(confirmed.len() * 16);
    for id in &confirmed {
        confirmed_bytes.extend_from_slice(id.as_bytes());
    }
    let hash = digest(&[
        b"decide",
        dossier.as_bytes(),
        snapshot.as_bytes(),
        decision.as_bytes(),
        &expected_bytes,
        reason.as_bytes(),
        &confirmed_bytes,
    ]);
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Accountant)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/review.decide/{}", key.as_str()))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record          WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='review.decide' AND key=$4",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(key.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((old, result)) = prior {
        if old != hash {
            return Err(Error::Conflict);
        }
        let mut row = review_by_dossier(&mut tx, t, c, dossier, a)
            .await?
            .filter(|row| row.id == result && row.decision != "PENDING")
            .ok_or(Error::Conflict)?;
        row.dossier_version = expected + 1;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(row);
    }
    let current = review_by_dossier(&mut tx, t, c, dossier, a)
        .await?
        .ok_or(Error::Conflict)?;
    if current.snapshot_id != snapshot || current.decision != "PENDING" {
        return Err(Error::Conflict);
    }
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let version: i64 = sqlx::query_scalar(
        "SELECT finance.workflow_decide_accounting_review(         $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    )
    .bind(t)
    .bind(c)
    .bind(dossier)
    .bind(expected)
    .bind(a)
    .bind(current.id)
    .bind(snapshot)
    .bind(decision)
    .bind(&confirmed)
    .bind(reason)
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_guard)?;
    sqlx::query(
        "INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id)          VALUES($1,$2,$3,'review.decide',$4,$5,$6)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(key.as_str())
    .bind(hash)
    .bind(current.id)
    .execute(&mut *tx)
    .await?;
    let mut row = review_by_dossier(&mut tx, t, c, dossier, a)
        .await?
        .filter(|row| row.id == current.id && row.decision == decision)
        .ok_or(Error::Conflict)?;
    row.dossier_version = version;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn get(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
) -> Result<Option<Review>, Error> {
    if dossier.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    let roles = lock_roles(&mut tx, proof.context()).await.map_err(grant)?;
    if !roles.contains(&Role::Requester) && !roles.contains(&Role::Accountant) {
        return Err(Error::Forbidden);
    }
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = review_by_dossier(&mut tx, t, c, dossier, a).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn queue(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Draft>, Error> {
    if !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Accountant)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows = sqlx::query_as::<_, Draft>(
        "SELECT * FROM finance.runtime_review_queue($1,$2,$3,$4,$5)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(after)
    .bind(i32::try_from(limit).map_err(|_| Error::InvalidInput)?)
    .fetch_all(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}

fn map_guard(error: sqlx::Error) -> Error {
    match &error {
        sqlx::Error::Database(database) if database.code().as_deref() == Some("23514") => {
            Error::Conflict
        }
        _ => Error::Database(error),
    }
}
