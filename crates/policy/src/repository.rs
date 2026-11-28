//! Authenticated read adapter for published policy versions.
use finance_core::prototype::{Error as AppError, recheck_authenticated, scoped_transaction};
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct ChecklistItem {
    pub id: Uuid,
    pub requirement: String,
    pub state: String,
    pub clause_id: Uuid,
    pub evidence_ids: Vec<Uuid>,
}

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
    let roles = lock_roles(&mut tx, proof.context()).await.map_err(grant)?;
    if !roles.contains(&Role::Requester)
        && !roles.contains(&Role::PolicyAdmin)
        && !roles.contains(&Role::FinanceApprover)
    {
        return Err(AppError::Forbidden);
    }
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows=sqlx::query_as::<_,PublishedPolicy>("SELECT id,company_id,aggregate_version AS version,COALESCE(display_name,dossier_kind||' policy') AS name,state,to_char(effective_from,'YYYY-MM-DD')||'T00:00:00Z' AS effective_from,to_char(effective_until,'YYYY-MM-DD')||'T00:00:00Z' AS effective_to,encode(source_digest,'hex') AS hash FROM finance.policy_version WHERE tenant_id=$1 AND company_id=$2 AND id>$3 ORDER BY id LIMIT $4")
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
    let roles = lock_roles(&mut tx, proof.context()).await.map_err(grant)?;
    if !roles.contains(&Role::Requester)
        && !roles.contains(&Role::PolicyAdmin)
        && !roles.contains(&Role::FinanceApprover)
    {
        return Err(AppError::Forbidden);
    }
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row=sqlx::query_as::<_,PublishedPolicy>("SELECT id,company_id,aggregate_version AS version,COALESCE(display_name,dossier_kind||' policy') AS name,state,to_char(effective_from,'YYYY-MM-DD')||'T00:00:00Z' AS effective_from,to_char(effective_until,'YYYY-MM-DD')||'T00:00:00Z' AS effective_to,encode(source_digest,'hex') AS hash FROM finance.policy_version WHERE tenant_id=$1 AND company_id=$2 AND id=$3")
        .bind(t).bind(c).bind(id).fetch_optional(&mut *tx).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn checklist(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    after: Uuid,
    limit: i64,
) -> Result<Option<Vec<ChecklistItem>>, AppError> {
    if dossier.is_nil() || !(1..=100).contains(&limit) {
        return Err(AppError::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => AppError::Forbidden,
            GrantError::Database(e) => AppError::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM finance.dossier WHERE tenant_id=$1 AND company_id=$2 AND owner_id=$3 AND id=$4)",
    )
    .bind(t).bind(c).bind(a).bind(dossier)
    .fetch_one(&mut *tx).await?;
    if !exists {
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(None);
    }
    let rows = sqlx::query_as::<_, ChecklistItem>(
        "SELECT * FROM finance.runtime_checklist($1,$2,$3,$4,$5,$6)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(dossier)
    .bind(after)
    .bind(i32::try_from(limit).map_err(|_| AppError::InvalidInput)?)
    .fetch_all(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(Some(rows))
}

fn grant(e: GrantError) -> AppError {
    match e {
        GrantError::Forbidden => AppError::Forbidden,
        GrantError::Database(e) => AppError::Database(e),
    }
}

#[derive(Clone, Copy)]
pub enum Change {
    Submit,
    Publish,
    Retire,
    Revoke,
    Revise,
}

impl Change {
    fn operation(self) -> &'static str {
        match self {
            Self::Submit => "policy.submit",
            Self::Publish => "policy.publish",
            Self::Retire => "policy.retire",
            Self::Revoke => "policy.revoke",
            Self::Revise => "policy.revise",
        }
    }
    fn role(self) -> Role {
        match self {
            Self::Submit | Self::Revise => Role::PolicyAdmin,
            _ => Role::FinanceApprover,
        }
    }
}

pub struct ChangeInput<'a> {
    pub policy: Uuid,
    pub new_id: Uuid,
    pub change: Change,
    pub reason: &'a str,
    pub name: Option<&'a str>,
    pub effective: Option<chrono::DateTime<chrono::Utc>>,
}

pub async fn change(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    input: ChangeInput<'_>,
) -> Result<PublishedPolicy, AppError> {
    let expected = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(AppError::InvalidInput)?,
    );
    if input.policy.is_nil()
        || input.new_id.is_nil()
        || input.reason.trim().is_empty()
        || input.reason.len() > 2048
        || input.reason.chars().any(char::is_control)
        || input.name.is_some_and(|name| {
            name.trim().is_empty() || name.len() > 200 || name.chars().any(char::is_control)
        })
        || input
            .effective
            .is_some_and(|value| value.time() != chrono::NaiveTime::MIN)
    {
        return Err(AppError::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), input.change.role())
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let row = sqlx::query_as::<_, PublishedPolicy>(
        "SELECT * FROM finance.runtime_policy_change($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::text::date,$11,$12,$13)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(input.policy)
    .bind(expected)
    .bind(input.change.operation())
    .bind(key.as_str())
    .bind(input.reason)
    .bind(input.name)
    .bind(input.effective.map(|value| value.date_naive().to_string()))
    .bind(input.new_id)
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await
    .map_err(
        |e| match e.as_database_error().and_then(|db| db.code()).as_deref() {
            Some("42501") => AppError::Forbidden,
            Some("22023") => AppError::InvalidInput,
            Some("23514" | "23505") => AppError::Conflict,
            _ => AppError::Database(e),
        },
    )?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

pub struct CreateDraft<'a> {
    pub id: Uuid,
    pub company: Uuid,
    pub name: &'a str,
    pub kind: &'a str,
    pub from: chrono::DateTime<chrono::Utc>,
    pub to: chrono::DateTime<chrono::Utc>,
    pub source: Uuid,
    pub clause_keys: &'a [String],
    pub evidence: &'a [Uuid],
    pub checklist_keys: &'a [String],
    pub document_kinds: &'a [String],
    pub mandatory: &'a [bool],
    pub critical_keys: &'a [String],
    pub critical_paths: &'a [String],
}

pub async fn create(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    input: CreateDraft<'_>,
) -> Result<PublishedPolicy, AppError> {
    let (t, c, a) = binding(proof)?;
    if input.company != c {
        return Err(AppError::Forbidden);
    }
    if input.id.is_nil()
        || input.source.is_nil()
        || input.name.trim().is_empty()
        || input.name.len() > 200
        || input.name.chars().any(char::is_control)
        || !matches!(input.kind, "PAYMENT" | "ADVANCE" | "SETTLEMENT")
        || input.from.time() != chrono::NaiveTime::MIN
        || input.to.time() != chrono::NaiveTime::MIN
        || input.from >= input.to
        || !(1..=200).contains(&input.clause_keys.len())
        || input.clause_keys.len() != input.evidence.len()
        || !(1..=200).contains(&input.checklist_keys.len())
        || input.checklist_keys.len() != input.document_kinds.len()
        || input.checklist_keys.len() != input.mandatory.len()
        || !input.mandatory.iter().any(|value| *value)
        || input.critical_paths.len()>200
        || input.critical_keys.len()!=input.critical_paths.len()
    {
        return Err(AppError::InvalidInput);
    }
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::PolicyAdmin)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let row = sqlx::query_as::<_, PublishedPolicy>(
        "SELECT * FROM finance.runtime_policy_create_configured($1,$2,$3,$4,$5,$6,$7,$8::text::date,$9::text::date,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(input.id)
    .bind(key.as_str())
    .bind(input.name)
    .bind(input.kind)
    .bind(input.from.date_naive().to_string())
    .bind(input.to.date_naive().to_string())
    .bind(input.source)
    .bind(input.clause_keys)
    .bind(input.evidence)
    .bind(input.checklist_keys)
    .bind(input.document_kinds)
    .bind(input.mandatory)
    .bind(command)
    .bind(correlation)
    .bind(input.critical_keys)
    .bind(input.critical_paths)
    .fetch_one(&mut *tx)
    .await
    .map_err(
        |e| match e.as_database_error().and_then(|db| db.code()).as_deref() {
            Some("42501") => AppError::Forbidden,
            Some("22023") => AppError::InvalidInput,
            Some("23514" | "23505") => AppError::Conflict,
            _ => AppError::Database(e),
        },
    )?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
