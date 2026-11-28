//! Public read port for analytics; keeps E24 from querying workflow tables directly.
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, require_role};
use finance_shared::{
    contracts::Principal,
    ids::{CompanyScope, ResourceScope},
};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, sqlx::FromRow)]
pub struct StateCounts {
    pub total: i64,
    pub draft: i64,
    pub processing: i64,
    pub needs_input: i64,
    pub reviewable: i64,
    pub ready: i64,
    pub exported: i64,
    pub handed_off: i64,
    pub accounting_review: i64,
    pub accepted: i64,
    pub returned: i64,
    pub cancelled: i64,
}

pub async fn state_counts(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
) -> Result<StateCounts, Error> {
    let ResourceScope::Company(CompanyScope { tenant, company }) = proof.context().scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(actor) = proof.context().actor else {
        return Err(Error::Forbidden);
    };
    let (t, c, a): (Uuid, Uuid, Uuid) = (tenant.into(), company.into(), actor.into());
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let counts = sqlx::query_as::<_, StateCounts>(
        "SELECT count(*)::bigint AS total,count(*) FILTER (WHERE status='DRAFT')::bigint AS draft,count(*) FILTER (WHERE status='PROCESSING')::bigint AS processing,count(*) FILTER (WHERE status='NEEDS_INPUT')::bigint AS needs_input,count(*) FILTER (WHERE status='REVIEWABLE')::bigint AS reviewable,count(*) FILTER (WHERE status='READY')::bigint AS ready,count(*) FILTER (WHERE status='EXPORTED')::bigint AS exported,count(*) FILTER (WHERE status='HANDED_OFF')::bigint AS handed_off,count(*) FILTER (WHERE status='ACCOUNTING_REVIEW')::bigint AS accounting_review,count(*) FILTER (WHERE status='ACCEPTED')::bigint AS accepted,count(*) FILTER (WHERE status='RETURNED')::bigint AS returned,count(*) FILTER (WHERE status='CANCELLED')::bigint AS cancelled FROM finance.dossier WHERE tenant_id=$1 AND company_id=$2 AND owner_id=$3",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .fetch_one(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(counts)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, sqlx::FromRow)]
pub struct ReviewHistoryCounts {
    pub accepted: i64,
    pub first_pass: i64,
    pub returned_cases: i64,
    pub unknown_accepted: i64,
}

/// Read an immutable review-history projection. Cases without a matched
/// decision are explicitly excluded from first-pass KPI, not treated as zero.
pub async fn review_history_counts(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
) -> Result<ReviewHistoryCounts, Error> {
    let ResourceScope::Company(CompanyScope { tenant, company }) = proof.context().scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(actor) = proof.context().actor else {
        return Err(Error::Forbidden);
    };
    let (t, c, a): (Uuid, Uuid, Uuid) = (tenant.into(), company.into(), actor.into());
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let counts = sqlx::query_as::<_, ReviewHistoryCounts>(
        "SELECT * FROM finance.runtime_review_history_counts($1,$2,$3)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .fetch_one(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(counts)
}

#[derive(sqlx::FromRow)]
struct DashboardCounts {
    #[sqlx(flatten)]
    state: StateCounts,
    review_accepted: i64,
    first_pass: i64,
    returned_cases: i64,
    unknown_accepted: i64,
}

pub async fn dashboard_counts(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
) -> Result<(StateCounts, ReviewHistoryCounts), Error> {
    let ResourceScope::Company(CompanyScope { tenant, company }) = proof.context().scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(actor) = proof.context().actor else {
        return Err(Error::Forbidden);
    };
    let (t, c, a): (Uuid, Uuid, Uuid) = (tenant.into(), company.into(), actor.into());
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    // Both projections share one PostgreSQL statement snapshot.
    let counts = sqlx::query_as::<_, DashboardCounts>(
        "SELECT state.*,review.accepted AS review_accepted,review.first_pass,review.returned_cases,review.unknown_accepted FROM (SELECT count(*)::bigint AS total,count(*) FILTER (WHERE status='DRAFT')::bigint AS draft,count(*) FILTER (WHERE status='PROCESSING')::bigint AS processing,count(*) FILTER (WHERE status='NEEDS_INPUT')::bigint AS needs_input,count(*) FILTER (WHERE status='REVIEWABLE')::bigint AS reviewable,count(*) FILTER (WHERE status='READY')::bigint AS ready,count(*) FILTER (WHERE status='EXPORTED')::bigint AS exported,count(*) FILTER (WHERE status='HANDED_OFF')::bigint AS handed_off,count(*) FILTER (WHERE status='ACCOUNTING_REVIEW')::bigint AS accounting_review,count(*) FILTER (WHERE status='ACCEPTED')::bigint AS accepted,count(*) FILTER (WHERE status='RETURNED')::bigint AS returned,count(*) FILTER (WHERE status='CANCELLED')::bigint AS cancelled FROM finance.dossier WHERE tenant_id=$1 AND company_id=$2 AND owner_id=$3) state CROSS JOIN finance.runtime_review_history_counts($1,$2,$3) review",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .fetch_one(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok((counts.state, ReviewHistoryCounts {
        accepted: counts.review_accepted,
        first_pass: counts.first_pass,
        returned_cases: counts.returned_cases,
        unknown_accepted: counts.unknown_accepted,
    }))
}
