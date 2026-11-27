//! Owner-scoped diagnostic reads. Synthetic rule version zero is not an approved rule set.
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

#[derive(Serialize, sqlx::FromRow)]
pub struct Check {
    pub id: Uuid,
    pub rule_id: String,
    pub rule_version: i64,
    pub outcome: String,
    pub severity: String,
    pub reason_code: String,
    pub evidence_ids: Vec<Uuid>,
    pub input_hash: String,
    pub explanation: String,
}

/// Latest run for the current revision and aggregate version only; a diagnostic PASS never authorizes a dossier.
/// Missing or inaccessible dossier is None, no run yet is an empty page.
pub async fn list(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    after: Uuid,
    limit: i64,
) -> Result<Option<Vec<Check>>, Error> {
    if dossier.is_nil() || !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
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
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM finance.dossier WHERE tenant_id=$1 AND company_id=$2 AND owner_id=$3 AND id=$4)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(dossier)
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        return Ok(None);
    }
    let rows = sqlx::query_as::<_, Check>(
        "SELECT x.id,x.rule_key AS rule_id,0::bigint AS rule_version,x.outcome,x.severity,x.detail_code AS reason_code,ARRAY[]::uuid[] AS evidence_ids,encode(r.input_hash,'hex') AS input_hash,'Synthetic preflight diagnostic; not an approved financial rule or readiness decision.'::text AS explanation FROM finance.check_result x JOIN finance.check_run r ON (r.tenant_id,r.company_id,r.id)=(x.tenant_id,x.company_id,x.run_id) WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.actor_id=$3 AND r.dossier_id=$4 AND r.id=(SELECT latest.id FROM finance.check_run latest JOIN finance.dossier d ON (d.tenant_id,d.company_id,d.id)=(latest.tenant_id,latest.company_id,latest.dossier_id) JOIN finance.dossier_revision dr ON (dr.tenant_id,dr.company_id,dr.dossier_id,dr.number,dr.id)=(d.tenant_id,d.company_id,d.id,d.revision,latest.revision_id) WHERE latest.tenant_id=$1 AND latest.company_id=$2 AND latest.actor_id=$3 AND latest.dossier_id=$4 AND latest.dossier_version=d.version ORDER BY latest.created_at DESC,latest.id DESC LIMIT 1) AND x.id>$5 ORDER BY x.id LIMIT $6",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(dossier)
    .bind(after)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(Some(rows))
}
