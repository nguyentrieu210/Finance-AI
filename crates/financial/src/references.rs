//! Owner-scoped reads of approved imported reference balances.
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, lock_roles, require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct ReferenceReceipt {
    pub id: Uuid,
    pub kind: String,
    pub source_key: String,
    pub as_of: String,
    pub coverage: String,
    pub state: String,
    pub evidence_ids: Vec<Uuid>,
    #[serde(skip)]
    pub version: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct ReferenceJob {
    pub id: Uuid,
    pub state: String,
    pub attempt: i64,
    pub progress: i64,
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

fn map_pg_error(error: sqlx::Error) -> Error {
    match &error {
        sqlx::Error::Database(db) => match db.code().as_deref() {
            Some("22023") => Error::InvalidInput,
            Some("42501") => Error::Forbidden,
            Some("23505") => Error::Conflict,
            _ => Error::Database(error),
        },
        _ => Error::Database(error),
    }
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
    if !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    let roles = lock_roles(&mut tx, proof.context())
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    if !roles.contains(&Role::Requester)
        && !roles.contains(&Role::PolicyAdmin)
        && !roles.contains(&Role::FinanceApprover)
    {
        return Err(Error::Forbidden);
    }
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let limit_i32 = i32::try_from(limit).map_err(|_| Error::InvalidInput)?;
    let rows = sqlx::query_as::<_, Reference>(
        "SELECT * FROM finance.runtime_list_references($1,$2,$3,$4,$5)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(after)
    .bind(limit_i32)
    .fetch_all(&mut *tx)
    .await
    .map_err(map_pg_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
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
    let (t, c, _) = binding(proof)?;
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
) -> Result<Option<ReferenceReceipt>, Error> {
    if id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    let roles = lock_roles(&mut tx, proof.context())
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    if !roles.contains(&Role::Requester)
        && !roles.contains(&Role::PolicyAdmin)
        && !roles.contains(&Role::FinanceApprover)
    {
        return Err(Error::Forbidden);
    }
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = sqlx::query_as::<_, ReferenceReceipt>(
        "SELECT * FROM finance.runtime_get_reference($1,$2,$3,$4)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(map_pg_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

#[derive(Clone, Copy, Debug)]
pub struct CreateReference<'a> {
    pub document_id: Uuid,
    pub kind: &'a str,
    pub as_of: &'a str,
    pub coverage: &'a str,
    pub reference_id: Uuid,
}

pub async fn create(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    input: CreateReference<'_>,
) -> Result<ReferenceJob, Error> {
    if input.document_id.is_nil()
        || input.reference_id.is_nil()
        || !matches!(
            input.kind,
            "VENDOR" | "CONTRACT" | "INVOICE" | "ADVANCE" | "BUDGET" | "PAYMENT"
        )
        || !matches!(input.coverage, "COMPLETE" | "PARTIAL" | "UNKNOWN")
        || input.as_of.is_empty()
    {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    let roles = lock_roles(&mut tx, proof.context())
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    if !roles.contains(&Role::Requester) && !roles.contains(&Role::PolicyAdmin) {
        return Err(Error::Forbidden);
    }
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = sqlx::query_as::<_, ReferenceJob>(
        "SELECT * FROM finance.runtime_create_reference($1,$2,$3,$4,$5,$6::text::timestamptz,$7,$8,$9,$10,$11)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(input.document_id)
    .bind(input.kind)
    .bind(input.as_of)
    .bind(input.coverage)
    .bind(key.as_str())
    .bind(input.reference_id)
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_pg_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn verify(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    reference_id: Uuid,
    reason: &str,
    evidence_ids: &[Uuid],
) -> Result<ReferenceReceipt, Error> {
    if reference_id.is_nil()
        || reason.trim().is_empty()
        || reason.len() > 4096
        || evidence_ids.is_empty()
        || evidence_ids.len() > 50
        || evidence_ids.iter().any(Uuid::is_nil)
    {
        return Err(Error::InvalidInput);
    }
    let expected = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(Error::InvalidInput)?,
    );
    let (t, c, a) = binding(proof)?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::FinanceApprover)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = sqlx::query_as::<_, ReferenceReceipt>(
        "SELECT * FROM finance.runtime_verify_reference($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(reference_id)
    .bind(expected)
    .bind(reason)
    .bind(evidence_ids)
    .bind(key.as_str())
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_pg_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
