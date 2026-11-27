//! Authenticated E17 retrieval over approved policy chunks.
//! No generation, rule mutation, or publication occurs here.
use chrono::NaiveDate;
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, require_role};
use finance_shared::{
    contracts::Principal,
    ids::{CompanyScope, ResourceScope},
    upload_contract::Sha256Digest,
};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct Hit {
    pub chunk_id: Uuid,
    pub policy_id: Uuid,
    pub clause_id: Uuid,
    pub excerpt: String,
    pub source_digest: String,
    pub source_page: i32,
    pub distance: f64,
}

fn valid_kind(value: &str) -> bool {
    matches!(value, "PAYMENT" | "ADVANCE" | "SETTLEMENT")
}
fn vector_text(vector: [f32; 3]) -> Option<String> {
    if vector.iter().any(|n| !n.is_finite()) || vector.iter().all(|n| *n == 0.0) {
        return None;
    }
    Some(format!("[{},{},{}]", vector[0], vector[1], vector[2]))
}

pub async fn retrieve(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    kind: &str,
    day: NaiveDate,
    query: &str,
    model_sha256: &str,
    vector: [f32; 3],
    limit: i64,
) -> Result<Vec<Hit>, Error> {
    if !valid_kind(kind)
        || query.trim().is_empty()
        || query.len() > 256
        || !(1..=20).contains(&limit)
        || Sha256Digest::parse(model_sha256).is_err()
    {
        return Err(Error::InvalidInput);
    }
    let Some(vector) = vector_text(vector) else {
        return Err(Error::InvalidInput);
    };
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
    let rows = sqlx::query_as::<_, Hit>(
        "SELECT x.id AS chunk_id,x.policy_id,x.clause_id,x.excerpt,encode(cl.source_digest,'hex') AS source_digest,cl.source_page,(x.embedding OPERATOR(finance.<=>) $7::text::finance.vector)::float8 AS distance
         FROM finance.knowledge_chunk x
         JOIN finance.policy_clause cl ON (cl.tenant_id,cl.company_id,cl.policy_id,cl.id)=(x.tenant_id,x.company_id,x.policy_id,x.clause_id)
         JOIN finance.policy_version p ON (p.tenant_id,p.company_id,p.id)=(x.tenant_id,x.company_id,x.policy_id)
         WHERE x.tenant_id=$1 AND x.company_id=$2 AND p.state='PUBLISHED' AND p.dossier_kind=$3
         AND p.effective_from<=$4::text::date AND $4::text::date<p.effective_until AND x.model_digest=decode($5,'hex')
         ORDER BY (position(lower($6) in lower(x.excerpt))>0) DESC,
                  x.embedding OPERATOR(finance.<=>) $7::text::finance.vector,x.id
         LIMIT $8",
    )
    .bind(t)
    .bind(c)
    .bind(kind)
    .bind(day.to_string())
    .bind(model_sha256)
    .bind(query)
    .bind(vector)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}
