//! E23 manual bridge and reconciliation repository. Live DO/ERP transport is deliberately absent.
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
pub struct Handoff {
    pub id: Uuid,
    pub package_id: Uuid,
    pub channel: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<String>,
    pub evidence_ids: Vec<Uuid>,
    #[serde(skip)]
    pub dossier_id: Uuid,
    #[serde(skip)]
    pub dossier_version: i64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct Reconciliation {
    pub id: Uuid,
    pub channel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<String>,
    pub state: String,
    pub reason: String,
    pub evidence_ids: Vec<Uuid>,
    #[serde(skip)]
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
fn grant(error: GrantError) -> Error {
    match error {
        GrantError::Forbidden => Error::Forbidden,
        GrantError::Database(error) => Error::Database(error),
    }
}
fn request_hash(parts: &[&[u8]]) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"finance.integration.command/v1");
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.finalize().to_vec()
}
fn validate_evidence(evidence: &[Uuid]) -> Result<Vec<Uuid>, Error> {
    if evidence.len() > 64 || evidence.iter().any(Uuid::is_nil) {
        return Err(Error::InvalidInput);
    }
    let mut sorted = evidence.to_vec();
    sorted.sort();
    sorted.dedup();
    if sorted.len() != evidence.len() {
        return Err(Error::InvalidInput);
    }
    Ok(sorted)
}
const SELECT_HANDOFF: &str = "SELECT h.id,h.package_id,h.channel,h.state,h.external_ref,h.evidence_ids,s.dossier_id,d.version AS dossier_version FROM finance.handoff h JOIN finance.dossier_snapshot s ON (s.tenant_id,s.company_id,s.id)=(h.tenant_id,h.company_id,h.snapshot_id) JOIN finance.dossier d ON (d.tenant_id,d.company_id,d.id)=(s.tenant_id,s.company_id,s.dossier_id)";
const SELECT_RECON: &str = "SELECT r.id,r.channel,h.external_ref,r.state,COALESCE(r.resolution_reason,r.reason) AS reason,CASE WHEN cardinality(r.resolution_evidence_ids)>0 THEN r.resolution_evidence_ids ELSE r.evidence_ids END AS evidence_ids,r.version FROM finance.reconciliation_case r JOIN finance.handoff h ON (h.tenant_id,h.company_id,h.id)=(r.tenant_id,r.company_id,r.handoff_id)";

async fn allowed_reconciliation_read(
    tx: &mut Transaction<'_, Postgres>,
    proof: &AuthenticatedCommand,
) -> Result<(), Error> {
    let roles = lock_roles(tx, proof.context()).await.map_err(grant)?;
    if roles.contains(&Role::Requester) || roles.contains(&Role::FinanceApprover) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

pub async fn list_handoffs(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Handoff>, Error> {
    if dossier.is_nil() || !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows = sqlx::query_as::<_, Handoff>(&format!(
        "{SELECT_HANDOFF} WHERE h.tenant_id=$1 AND h.company_id=$2 AND s.dossier_id=$3 AND h.id>$4 ORDER BY h.id LIMIT $5"
    ))
    .bind(tenant)
    .bind(company)
    .bind(dossier)
    .bind(after)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn create_manual_handoff(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    dossier: Uuid,
    handoff: Uuid,
    package: Uuid,
    external_ref: &str,
    evidence: &[Uuid],
) -> Result<Handoff, Error> {
    let Some(expected) = proof.context().expected_version else {
        return Err(Error::InvalidInput);
    };
    let expected = i64::from(expected);
    if dossier.is_nil()
        || handoff.is_nil()
        || package.is_nil()
        || external_ref.trim() != external_ref
        || external_ref.is_empty()
        || external_ref.len() > 256
    {
        return Err(Error::InvalidInput);
    }
    let evidence = validate_evidence(evidence)?;
    let mut evidence_bytes = Vec::with_capacity(evidence.len() * 16);
    for id in &evidence {
        evidence_bytes.extend_from_slice(id.as_bytes());
    }
    let expected_bytes = expected.to_be_bytes();
    let digest = request_hash(&[
        dossier.as_bytes(),
        package.as_bytes(),
        &expected_bytes,
        external_ref.as_bytes(),
        &evidence_bytes,
    ]);
    let (tenant, company, actor) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "{tenant}/{company}/{actor}/handoff.manual/{}",
            key.as_str()
        ))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='handoff.manual' AND key=$4",
    )
    .bind(tenant)
    .bind(company)
    .bind(actor)
    .bind(key.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((old, result)) = prior {
        if old != digest {
            return Err(Error::Conflict);
        }
        let row = sqlx::query_as::<_, Handoff>(&format!(
            "{SELECT_HANDOFF} WHERE h.tenant_id=$1 AND h.company_id=$2 AND h.id=$3"
        ))
        .bind(tenant)
        .bind(company)
        .bind(result)
        .fetch_one(&mut *tx)
        .await?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(row);
    }
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query_scalar::<_, i64>(
        "SELECT finance.integration_manual_handoff($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
    )
    .bind(tenant)
    .bind(company)
    .bind(dossier)
    .bind(expected)
    .bind(actor)
    .bind(package)
    .bind(handoff)
    .bind(external_ref)
    .bind(&evidence)
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'handoff.manual',$4,$5,$6)",
    )
    .bind(tenant)
    .bind(company)
    .bind(actor)
    .bind(key.as_str())
    .bind(digest)
    .bind(handoff)
    .execute(&mut *tx)
    .await?;
    let row = sqlx::query_as::<_, Handoff>(&format!(
        "{SELECT_HANDOFF} WHERE h.tenant_id=$1 AND h.company_id=$2 AND h.id=$3"
    ))
    .bind(tenant)
    .bind(company)
    .bind(handoff)
    .fetch_one(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn list_reconciliations(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Reconciliation>, Error> {
    if !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    allowed_reconciliation_read(&mut tx, proof).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows = sqlx::query_as::<_, Reconciliation>(&format!(
        "{SELECT_RECON} WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.id>$3 ORDER BY r.id LIMIT $4"
    ))
    .bind(tenant)
    .bind(company)
    .bind(after)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}
pub async fn get_reconciliation(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    id: Uuid,
) -> Result<Option<Reconciliation>, Error> {
    if id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    allowed_reconciliation_read(&mut tx, proof).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = sqlx::query_as::<_, Reconciliation>(&format!(
        "{SELECT_RECON} WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.id=$3"
    ))
    .bind(tenant)
    .bind(company)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
pub async fn resolve_reconciliation(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    id: Uuid,
    reason: &str,
    evidence: &[Uuid],
) -> Result<Reconciliation, Error> {
    let Some(expected) = proof.context().expected_version else {
        return Err(Error::InvalidInput);
    };
    let expected = i64::from(expected);
    if id.is_nil()
        || reason.trim().is_empty()
        || reason.len() > 4096
        || reason.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput);
    }
    let evidence = validate_evidence(evidence)?;
    let mut evidence_bytes = Vec::with_capacity(evidence.len() * 16);
    for value in &evidence {
        evidence_bytes.extend_from_slice(value.as_bytes());
    }
    let expected_bytes = expected.to_be_bytes();
    let digest = request_hash(&[id.as_bytes(), &expected_bytes, reason.as_bytes(), &evidence_bytes]);
    let (tenant, company, actor) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::FinanceApprover)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "{tenant}/{company}/{actor}/reconciliation.resolve/{}",
            key.as_str()
        ))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='reconciliation.resolve' AND key=$4",
    )
    .bind(tenant)
    .bind(company)
    .bind(actor)
    .bind(key.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((old, result)) = prior {
        if old != digest || result != id {
            return Err(Error::Conflict);
        }
        let row = sqlx::query_as::<_, Reconciliation>(&format!(
            "{SELECT_RECON} WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.id=$3"
        ))
        .bind(tenant)
        .bind(company)
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(row);
    }
    let affected = sqlx::query(
        "UPDATE finance.reconciliation_case SET state='RESOLVED',assigned_actor=$4,resolved_at=clock_timestamp(),resolution_reason=$5,resolution_evidence_ids=$6,version=version+1 WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND state='OPEN' AND version=$7",
    )
    .bind(tenant)
    .bind(company)
    .bind(id)
    .bind(actor)
    .bind(reason)
    .bind(&evidence)
    .bind(expected)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if affected != 1 {
        return Err(Error::Conflict);
    }
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query(
        "INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'RECONCILIATION_RESOLVED')",
    )
    .bind(tenant)
    .bind(company)
    .bind(command)
    .bind(actor)
    .bind(correlation)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'RECONCILIATION_RESOLVED')",
    )
    .bind(tenant)
    .bind(company)
    .bind(command)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'reconciliation.resolve',$4,$5,$6)",
    )
    .bind(tenant)
    .bind(company)
    .bind(actor)
    .bind(key.as_str())
    .bind(digest)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    let row = sqlx::query_as::<_, Reconciliation>(&format!(
        "{SELECT_RECON} WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.id=$3"
    ))
    .bind(tenant)
    .bind(company)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
