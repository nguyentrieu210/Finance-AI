//! E11 authenticated read projection over reservation state.
//! Reservation mutation remains an internal finance transaction port.
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, require_role};
use finance_shared::http_contract::IdempotencyKey;
use finance_shared::{
    contracts::Principal,
    ids::{CompanyScope, ResourceScope},
};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct ReservationRow {
    pub id: Uuid,
    pub snapshot_id: Option<Uuid>,
    pub resource_id: Uuid,
    pub resource_ids: Vec<Uuid>,
    pub revision_id: Uuid,
    pub policy_id: Option<Uuid>,
    pub amount: String,
    pub remaining: String,
    pub currency: String,
    pub state: String,
    pub version: i64,
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
fn map_database(error: sqlx::Error) -> Error {
    if error
        .as_database_error()
        .and_then(|value| value.code())
        .as_deref()
        == Some("42501")
    {
        Error::Forbidden
    } else {
        Error::Database(error)
    }
}
pub async fn list(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    after: Uuid,
    limit: i64,
) -> Result<Vec<ReservationRow>, Error> {
    if dossier.is_nil() || !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, actor) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    let roles = finance_permissions::lock_roles(&mut tx, proof.context())
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    if !roles.contains(&Role::Requester)
        && !roles.contains(&Role::FinanceApprover)
        && !roles.contains(&Role::Accountant)
    {
        return Err(Error::Forbidden);
    }
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows = sqlx::query_as::<_, ReservationRow>(
        "SELECT * FROM finance.runtime_reservation_ledger($1,$2,$3,$4,$5,$6::integer)",
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

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct BindingReceipt {
    pub id: Uuid,
    pub dossier_version: i64,
}
pub async fn bind(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    dossier: Uuid,
    policy: Uuid,
    resources: &[Uuid],
    reason: &str,
    id: Uuid,
) -> Result<BindingReceipt, Error> {
    let version = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(Error::InvalidInput)?,
    );
    let (t, c, a) = binding(proof)?;
    if dossier.is_nil()
        || policy.is_nil()
        || id.is_nil()
        || !(1..=32).contains(&resources.len())
        || resources.iter().any(Uuid::is_nil)
        || reason.trim().is_empty()
        || reason.len() > 2048
    {
        return Err(Error::InvalidInput);
    }
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::FinanceApprover)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let row = sqlx::query_as::<_, BindingReceipt>(
        "SELECT * FROM finance.runtime_bind_reservation($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(dossier)
    .bind(version)
    .bind(policy)
    .bind(resources)
    .bind(reason)
    .bind(key.as_str())
    .bind(id)
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await
    .map_err(
        |e| match e.as_database_error().and_then(|d| d.code()).as_deref() {
            Some("42501") => Error::Forbidden,
            Some("22023") => Error::InvalidInput,
            Some("23514" | "23505") => Error::Conflict,
            _ => Error::Database(e),
        },
    )?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn release(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    dossier: Uuid,
    reservation: Uuid,
    reason: &str,
) -> Result<BindingReceipt, Error> {
    let version = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(Error::InvalidInput)?,
    );
    let (t, c, a) = binding(proof)?;
    if dossier.is_nil() || reservation.is_nil() || reason.trim().is_empty() || reason.len() > 2048 {
        return Err(Error::InvalidInput);
    }
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::FinanceApprover)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let row=sqlx::query_as::<_,BindingReceipt>("SELECT * FROM finance.runtime_release_unsealed_reservation($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
        .bind(t).bind(c).bind(a).bind(dossier).bind(version).bind(reservation).bind(reason).bind(key.as_str()).bind(command).bind(correlation)
        .fetch_one(&mut *tx).await.map_err(|e|match e.as_database_error().and_then(|d|d.code()).as_deref(){Some("42501")=>Error::Forbidden,Some("22023")=>Error::InvalidInput,Some("23514"|"23505")=>Error::Conflict,_=>Error::Database(e)})?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
