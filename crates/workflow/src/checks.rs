//! Owner-scoped diagnostic reads. Synthetic rule version zero is not an approved rule set.
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
    ids::{CompanyScope, ResourceScope},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
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

pub struct Acknowledge<'a> {
    pub dossier: Uuid,
    pub result: Uuid,
    pub acknowledgment: Uuid,
    pub reason: &'a str,
}

pub async fn acknowledge(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    input: Acknowledge<'_>,
) -> Result<(Check, i64), Error> {
    let version = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(Error::InvalidInput)?,
    );
    if input.dossier.is_nil()
        || input.result.is_nil()
        || input.acknowledgment.is_nil()
        || input.reason.trim().is_empty()
        || input.reason.len() > 2048
        || input.reason.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput);
    }
    let ResourceScope::Company(CompanyScope { tenant, company }) = proof.context().scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(actor) = proof.context().actor else {
        return Err(Error::Forbidden);
    };
    let (t, c, a): (Uuid, Uuid, Uuid) = (tenant.into(), company.into(), actor.into());
    let mut hash = Sha256::new();
    hash.update(b"finance.warning.ack/v1");
    hash.update(input.dossier.as_bytes());
    hash.update(input.result.as_bytes());
    hash.update(version.to_be_bytes());
    hash.update(input.reason.as_bytes());
    let hash = hash.finalize().to_vec();
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/warning.ack/{}", key.as_str()))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='warning.ack' AND key=$4",
    ).bind(t).bind(c).bind(a).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    let ack = if let Some((old, ack)) = prior {
        if old != hash {
            return Err(Error::Conflict);
        }
        ack
    } else {
        let command: Uuid = proof.context().command.into();
        let correlation: Uuid = proof.context().correlation.into();
        sqlx::query("SELECT finance.workflow_acknowledge_warning($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)")
            .bind(t)
            .bind(c)
            .bind(a)
            .bind(input.dossier)
            .bind(version)
            .bind(input.result)
            .bind(input.acknowledgment)
            .bind(input.reason)
            .bind(command)
            .bind(correlation)
            .execute(&mut *tx)
            .await
            .map_err(map_ack_error)?;
        sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_version) VALUES($1,$2,$3,'warning.ack',$4,$5,$6,$7)")
            .bind(t).bind(c).bind(a).bind(key.as_str()).bind(hash).bind(input.acknowledgment)
            .bind(version).execute(&mut *tx).await?;
        input.acknowledgment
    };
    let receipt = sqlx::query_as::<_, Check>(
        "SELECT * FROM finance.runtime_warning_receipt($1,$2,$3,$4,$5,$6)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(input.dossier)
    .bind(ack)
    .bind(version)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(Error::Forbidden)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok((receipt, version))
}

fn map_ack_error(error: sqlx::Error) -> Error {
    if let sqlx::Error::Database(ref db) = error {
        if matches!(db.code().as_deref(), Some("23514" | "23505")) {
            return Error::Conflict;
        }
    }
    Error::Database(error)
}

#[derive(Serialize)]
pub struct PreflightJob {
    pub id: Uuid,
    pub state: &'static str,
    pub attempt: i64,
    pub progress: i64,
    #[serde(skip)]
    pub version: i64,
}

pub async fn request_preflight(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    dossier: Uuid,
    policy: Uuid,
    run: Uuid,
) -> Result<PreflightJob, Error> {
    let version = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(Error::InvalidInput)?,
    );
    if dossier.is_nil() || policy.is_nil() || run.is_nil() {
        return Err(Error::InvalidInput);
    }
    let ResourceScope::Company(CompanyScope { tenant, company }) = proof.context().scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(actor) = proof.context().actor else {
        return Err(Error::Forbidden);
    };
    let (t, c, a): (Uuid, Uuid, Uuid) = (tenant.into(), company.into(), actor.into());
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let id: Uuid = sqlx::query_scalar(
        "SELECT finance.runtime_request_preflight($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(dossier)
    .bind(version)
    .bind(policy)
    .bind(run)
    .bind(command)
    .bind(correlation)
    .bind(key.as_str())
    .fetch_one(&mut *tx)
    .await
    .map_err(
        |e| match e.as_database_error().and_then(|db| db.code()).as_deref() {
            Some("42501") => Error::Forbidden,
            Some("22023") => Error::InvalidInput,
            Some("23514" | "23505") => Error::Conflict,
            _ => Error::Database(e),
        },
    )?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(PreflightJob {
        id,
        state: "QUEUED",
        attempt: 0,
        progress: 0,
        version,
    })
}
