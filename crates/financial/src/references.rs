//! Owner-scoped reads of approved imported reference balances.
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceKind {
    Contract,
    Advance,
    Budget,
}
impl ReferenceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Contract => "CONTRACT",
            Self::Advance => "ADVANCE",
            Self::Budget => "BUDGET",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct Reference {
    pub id: Uuid,
    pub kind: String,
    pub source_key: String,
    pub as_of: String,
    pub coverage: String,
    pub state: String,
    pub evidence_ids: Vec<Uuid>,
}

pub async fn list(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    kind: ReferenceKind,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Reference>, Error> {
    list_filtered(pool, proof, Some(kind), after, limit).await
}

/// List verified, fresh financial reference resources in the authorized company.
pub async fn list_all(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Reference>, Error> {
    list_filtered(pool, proof, None, after, limit).await
}

async fn list_filtered(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    kind: Option<ReferenceKind>,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Reference>, Error> {
    if !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let ResourceScope::Company(CompanyScope { tenant, company }) = proof.context().scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(_) = proof.context().actor else {
        return Err(Error::Forbidden);
    };
    let (t, c): (Uuid, Uuid) = (tenant.into(), company.into());
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows = sqlx::query_as::<_, Reference>(
        "SELECT f.id,f.resource_kind AS kind,f.source_key,to_char(r.as_of AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS as_of,'COMPLETE'::text AS coverage,'VERIFIED'::text AS state,ARRAY[]::uuid[] AS evidence_ids FROM finance.financial_resource f JOIN finance.reference_snapshot r ON (r.tenant_id,r.company_id,r.id)=(f.tenant_id,f.company_id,f.snapshot_id) WHERE f.tenant_id=$1 AND f.company_id=$2 AND ($3::text IS NULL OR f.resource_kind=$3) AND f.resource_kind IN ('CONTRACT','ADVANCE','BUDGET','INVOICE') AND f.id>$4 AND f.coverage_complete AND f.fresh_until>statement_timestamp() AND r.state='APPROVED' AND r.fresh_until>statement_timestamp() ORDER BY f.id LIMIT $5",
    )
    .bind(t)
    .bind(c)
    .bind(kind.map(ReferenceKind::as_str))
    .bind(after)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn list_contracts(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Reference>, Error> {
    list(pool, proof, ReferenceKind::Contract, after, limit).await
}
pub async fn list_advances(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Reference>, Error> {
    list(pool, proof, ReferenceKind::Advance, after, limit).await
}
pub async fn list_budgets(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Reference>, Error> {
    list(pool, proof, ReferenceKind::Budget, after, limit).await
}

pub async fn get(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    id: Uuid,
) -> Result<Option<Reference>, Error> {
    if id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let ResourceScope::Company(CompanyScope { tenant, company }) = proof.context().scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(_) = proof.context().actor else {
        return Err(Error::Forbidden);
    };
    let (t, c): (Uuid, Uuid) = (tenant.into(), company.into());
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = sqlx::query_as::<_, Reference>(
        "SELECT f.id,f.resource_kind AS kind,f.source_key,to_char(r.as_of AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS as_of,'COMPLETE'::text AS coverage,'VERIFIED'::text AS state,ARRAY[]::uuid[] AS evidence_ids FROM finance.financial_resource f JOIN finance.reference_snapshot r ON (r.tenant_id,r.company_id,r.id)=(f.tenant_id,f.company_id,f.snapshot_id) WHERE f.tenant_id=$1 AND f.company_id=$2 AND f.id=$3 AND f.resource_kind IN ('CONTRACT','ADVANCE','BUDGET','INVOICE') AND f.coverage_complete AND f.fresh_until>statement_timestamp() AND r.state='APPROVED' AND r.fresh_until>statement_timestamp()",
    )
    .bind(t)
    .bind(c)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
