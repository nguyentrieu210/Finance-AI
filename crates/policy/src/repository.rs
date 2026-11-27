//! Authenticated read adapter for published policy versions.
use finance_core::prototype::{Error as AppError, recheck_authenticated, scoped_transaction};
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
pub struct PublishedPolicy {
    pub id: Uuid,
    pub company_id: Uuid,
    pub version: i64,
    pub name: String,
    pub state: String,
    pub effective_from: String,
    pub effective_to: String,
    pub hash: String,
}

fn binding(proof: &AuthenticatedCommand) -> Result<(Uuid, Uuid, Uuid), AppError> {
    let ResourceScope::Company(CompanyScope { tenant, company }) = proof.context().scope else {
        return Err(AppError::Forbidden);
    };
    let Principal::User(actor) = proof.context().actor else {
        return Err(AppError::Forbidden);
    };
    Ok((tenant.into(), company.into(), actor.into()))
}

pub async fn list(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<PublishedPolicy>, AppError> {
    if !(1..=100).contains(&limit) {
        return Err(AppError::InvalidInput);
    }
    let (t, c, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => AppError::Forbidden,
            GrantError::Database(e) => AppError::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows=sqlx::query_as::<_,PublishedPolicy>("SELECT id,company_id,1::bigint AS version,dossier_kind||' policy' AS name,state,to_char(effective_from,'YYYY-MM-DD')||'T00:00:00Z' AS effective_from,to_char(effective_until,'YYYY-MM-DD')||'T00:00:00Z' AS effective_to,encode(source_digest,'hex') AS hash FROM finance.policy_version WHERE tenant_id=$1 AND company_id=$2 AND state='PUBLISHED' AND id>$3 ORDER BY id LIMIT $4")
        .bind(t).bind(c).bind(after).bind(limit).fetch_all(&mut *tx).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn get(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    id: Uuid,
) -> Result<Option<PublishedPolicy>, AppError> {
    if id.is_nil() {
        return Err(AppError::InvalidInput);
    }
    let (t, c, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => AppError::Forbidden,
            GrantError::Database(e) => AppError::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row=sqlx::query_as::<_,PublishedPolicy>("SELECT id,company_id,1::bigint AS version,dossier_kind||' policy' AS name,state,to_char(effective_from,'YYYY-MM-DD')||'T00:00:00Z' AS effective_from,to_char(effective_until,'YYYY-MM-DD')||'T00:00:00Z' AS effective_to,encode(source_digest,'hex') AS hash FROM finance.policy_version WHERE tenant_id=$1 AND company_id=$2 AND state='PUBLISHED' AND id=$3")
        .bind(t).bind(c).bind(id).fetch_optional(&mut *tx).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
