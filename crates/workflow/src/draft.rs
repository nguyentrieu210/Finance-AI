//! Authenticated draft creation and owner reads in the synthetic environment.
//! No policy publication, document attachment, readiness or approval.
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
    ids::{CompanyScope, ResourceScope},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Payment,
    Advance,
    Settlement,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Payment => "PAYMENT",
            Self::Advance => "ADVANCE",
            Self::Settlement => "SETTLEMENT",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct Draft {
    pub id: Uuid,
    pub company_id: Uuid,
    pub code: String,
    pub kind: String,
    pub status: String,
    pub revision: i64,
    pub version: i64,
    pub owner_id: Uuid,
    pub updated_at: String,
}

const SELECT_DRAFT: &str = "SELECT id,company_id,code,kind,status,revision,version,owner_id,to_char(updated_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at FROM finance.dossier";

fn binding(proof: &AuthenticatedCommand) -> Result<(Uuid, Uuid, Uuid), Error> {
    let ResourceScope::Company(CompanyScope { tenant, company }) = proof.context().scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(actor) = proof.context().actor else {
        return Err(Error::Forbidden);
    };
    Ok((tenant.into(), company.into(), actor.into()))
}

fn grant_error(error: GrantError) -> Error {
    match error {
        GrantError::Forbidden => Error::Forbidden,
        GrantError::Database(error) => Error::Database(error),
    }
}

pub async fn create(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    kind: Kind,
    id: Uuid,
    revision_id: Uuid,
) -> Result<Draft, Error> {
    if id.is_nil()
        || revision_id.is_nil()
        || id == revision_id
        || proof.context().expected_version.is_some()
    {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    let lock = format!("{t}/{c}/{a}/prototype.createDossier/{}", key.as_str());
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(lock)
        .execute(&mut *tx)
        .await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(grant_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let hash =
        Sha256::digest(format!("finance.createDossier/v1/{t}/{c}/{}", kind.as_str()).as_bytes())
            .to_vec();
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as("SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='prototype.createDossier' AND key=$4")
        .bind(t).bind(c).bind(a).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    let result_id = if let Some((old_hash, old_id)) = prior {
        if old_hash != hash {
            return Err(Error::Conflict);
        }
        old_id
    } else {
        let code = format!("SYN-{id}");
        sqlx::query("INSERT INTO finance.dossier(tenant_id,company_id,id,owner_id,code,kind) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(t).bind(c).bind(id).bind(a).bind(code).bind(kind.as_str()).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO finance.dossier_revision(tenant_id,company_id,dossier_id,id,owner_id,number) VALUES($1,$2,$3,$4,$5,0)")
            .bind(t).bind(c).bind(id).bind(revision_id).bind(a).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'DOSSIER_CREATED')")
            .bind(t).bind(c).bind(command).bind(a).bind(correlation).bind(id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'DOSSIER_CREATED')")
            .bind(t).bind(c).bind(command).bind(id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'prototype.createDossier',$4,$5,$6)")
            .bind(t).bind(c).bind(a).bind(key.as_str()).bind(hash).bind(id).execute(&mut *tx).await?;
        id
    };
    let draft = sqlx::query_as::<_, Draft>(r#"SELECT id,company_id,code,kind,'DRAFT'::text AS status,0::bigint AS revision,0::bigint AS version,owner_id,to_char(initial_updated_at AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS updated_at FROM finance.dossier WHERE tenant_id=$1 AND company_id=$2 AND owner_id=$3 AND id=$4"#)
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(result_id)
    .fetch_one(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(draft)
}

pub async fn get(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    id: Uuid,
) -> Result<Option<Draft>, Error> {
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(grant_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let draft = sqlx::query_as::<_, Draft>(&format!(
        "{SELECT_DRAFT} WHERE tenant_id=$1 AND company_id=$2 AND owner_id=$3 AND id=$4"
    ))
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(draft)
}

#[derive(Default)]
pub struct Filters<'a> {
    pub company_id: Option<Uuid>,
    pub kind: Option<&'a str>,
    pub status: Option<&'a str>,
    pub updated_from: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_to: Option<chrono::DateTime<chrono::Utc>>,
}

impl Filters<'_> {
    pub fn valid(&self) -> bool {
        self.kind
            .is_none_or(|k| matches!(k, "PAYMENT" | "ADVANCE" | "SETTLEMENT"))
            && self.status.is_none_or(|s| {
                matches!(
                    s,
                    "DRAFT"
                        | "PROCESSING"
                        | "NEEDS_INPUT"
                        | "REVIEWABLE"
                        | "READY"
                        | "EXPORTED"
                        | "HANDED_OFF"
                        | "ACCOUNTING_REVIEW"
                        | "ACCEPTED"
                        | "RETURNED"
                        | "CANCELLED"
                )
            })
            && self
                .updated_from
                .zip(self.updated_to)
                .is_none_or(|(from, to)| from <= to)
    }
}

pub async fn list(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Draft>, Error> {
    list_filtered(pool, proof, after, limit, &Filters::default()).await
}

pub async fn list_filtered(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
    filters: &Filters<'_>,
) -> Result<Vec<Draft>, Error> {
    if !(1..=100).contains(&limit) || !filters.valid() {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    if filters.company_id.is_some_and(|id| id != c) {
        return Err(Error::Forbidden);
    }
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(grant_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let drafts = sqlx::query_as::<_, Draft>(&format!(
        "{SELECT_DRAFT} WHERE tenant_id=$1 AND company_id=$2 AND owner_id=$3 AND id>$4 AND ($6::text IS NULL OR kind=$6) AND ($7::text IS NULL OR status=$7) AND ($8::text::timestamptz IS NULL OR updated_at >= $8::text::timestamptz) AND ($9::text::timestamptz IS NULL OR updated_at <= $9::text::timestamptz) ORDER BY id LIMIT $5"
    ))
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(after)
    .bind(limit)
    .bind(filters.kind)
    .bind(filters.status)
    .bind(filters.updated_from.map(|date| date.to_rfc3339()))
    .bind(filters.updated_to.map(|date| date.to_rfc3339()))
    .fetch_all(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(drafts)
}

#[derive(Clone, Copy)]
pub enum Change {
    Revise,
    Reopen,
    Cancel,
}

impl Change {
    fn operation(self) -> &'static str {
        match self {
            Self::Revise => "dossier.revise",
            Self::Reopen => "dossier.reopen",
            Self::Cancel => "dossier.cancel",
        }
    }
}

pub struct ChangeInput<'a> {
    pub dossier: Uuid,
    pub revision: Uuid,
    pub change: Change,
    pub reason: &'a str,
    pub evidence: &'a [Uuid],
}

pub async fn change(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    input: ChangeInput<'_>,
) -> Result<Draft, Error> {
    let expected = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(Error::InvalidInput)?,
    );
    if input.dossier.is_nil()
        || input.revision.is_nil()
        || input.reason.trim().is_empty()
        || input.reason.len() > 2048
        || input.reason.chars().any(char::is_control)
        || input.evidence.len() > 64
        || input.evidence.iter().any(Uuid::is_nil)
    {
        return Err(Error::InvalidInput);
    }
    let mut evidence = input.evidence.to_vec();
    evidence.sort();
    evidence.dedup();
    if evidence.len() != input.evidence.len() {
        return Err(Error::InvalidInput);
    }
    let operation = input.change.operation();
    let mut digest = Sha256::new();
    digest.update(b"finance.dossier.change/v1");
    digest.update(operation.as_bytes());
    digest.update(input.dossier.as_bytes());
    digest.update(expected.to_be_bytes());
    digest.update((input.reason.len() as u64).to_be_bytes());
    digest.update(input.reason.as_bytes());
    for id in &evidence {
        digest.update(id.as_bytes());
    }
    let hash = digest.finalize().to_vec();
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(grant_error)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/{operation}/{}", key.as_str()))
        .execute(&mut *tx)
        .await?;
    let owned: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM finance.dossier WHERE tenant_id=$1 AND company_id=$2 AND owner_id=$3 AND id=$4)",
    ).bind(t).bind(c).bind(a).bind(input.dossier).fetch_one(&mut *tx).await?;
    if !owned {
        return Err(Error::Forbidden);
    }
    let prior: Option<(Vec<u8>, Uuid, String)> = sqlx::query_as(
        "SELECT request_hash,result_id,(result_payload->'dossier')::text AS receipt FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id=$4 AND key=$5",
    ).bind(t).bind(c).bind(a).bind(operation).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    let row = if let Some((old, id, receipt)) = prior {
        if old != hash || id != input.dossier {
            return Err(Error::Conflict);
        }
        serde_json::from_str(&receipt).map_err(|_| Error::Conflict)?
    } else {
        let command: Uuid = proof.context().command.into();
        let correlation: Uuid = proof.context().correlation.into();
        sqlx::query("SELECT finance.workflow_change_dossier($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
            .bind(t)
            .bind(c)
            .bind(a)
            .bind(input.dossier)
            .bind(expected)
            .bind(operation)
            .bind(input.revision)
            .bind(input.reason)
            .bind(&evidence)
            .bind(command)
            .bind(correlation)
            .execute(&mut *tx)
            .await
            .map_err(map_change_error)?;
        let row = sqlx::query_as::<_, Draft>(&format!(
            "{SELECT_DRAFT} WHERE tenant_id=$1 AND company_id=$2 AND owner_id=$3 AND id=$4"
        ))
        .bind(t)
        .bind(c)
        .bind(a)
        .bind(input.dossier)
        .fetch_one(&mut *tx)
        .await?;
        let payload = serde_json::json!({
            "dossier": row,
            "reason": input.reason,
            "evidence_ids": evidence
        })
        .to_string();
        sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_version,result_payload) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9::text::jsonb)")
            .bind(t).bind(c).bind(a).bind(operation).bind(key.as_str()).bind(hash)
            .bind(input.dossier).bind(row.version).bind(payload).execute(&mut *tx).await?;
        row
    };
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

fn map_change_error(error: sqlx::Error) -> Error {
    if let sqlx::Error::Database(ref db) = error {
        if matches!(db.code().as_deref(), Some("23514" | "23505")) {
            return Error::Conflict;
        }
    }
    Error::Database(error)
}
