//! E08 authenticated read adapter over current persisted extraction/evidence.
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, lock_roles};
use finance_shared::{
    contracts::Principal,
    ids::{CompanyScope, ResourceScope},
};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct Field {
    pub id: Uuid,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_value: Option<String>,
    pub value_type: String,
    pub state: String,
    pub evidence_ids: Vec<Uuid>,
    pub extractor_version: String,
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

async fn require_reader(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    proof: &AuthenticatedCommand,
) -> Result<(), Error> {
    let roles = lock_roles(tx, proof.context()).await.map_err(grant)?;
    if roles.contains(&Role::Requester) || roles.contains(&Role::Accountant) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
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
) -> Result<Vec<Field>, Error> {
    if dossier.is_nil() || !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, actor) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_reader(&mut tx, proof).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows = sqlx::query_as::<_, Field>(
        "SELECT * FROM finance.runtime_fields($1,$2,$3,$4,$5,NULL,$6::integer)",
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

pub async fn get(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    field: Uuid,
) -> Result<Option<Field>, Error> {
    if dossier.is_nil() || field.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, actor) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_reader(&mut tx, proof).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row =
        sqlx::query_as::<_, Field>("SELECT * FROM finance.runtime_fields($1,$2,$3,$4,$5,$6,1)")
            .bind(tenant)
            .bind(company)
            .bind(dossier)
            .bind(actor)
            .bind(Uuid::nil())
            .bind(field)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_database)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
