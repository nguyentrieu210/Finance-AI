//! E11 authenticated read projection over reservation state.
//! Reservation mutation remains an internal finance transaction port.
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, require_role};
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
    pub snapshot_id: Uuid,
    pub resource_id: Uuid,
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
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|error| match error {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(error) => Error::Database(error),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows = sqlx::query_as::<_, ReservationRow>(
        "SELECT * FROM finance.runtime_reservations($1,$2,$3,$4,$5,$6)",
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
