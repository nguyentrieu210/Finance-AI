//! E09 authenticated independent field review with typed validation and immutable receipts.
use crate::{
    Outcome,
    schema::{FieldType, validate},
};
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_extraction::repository::Field;
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
    ids::{CompanyScope, ResourceScope},
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct ReceiptRow {
    id: Uuid,
    path: String,
    raw_value: Option<String>,
    normalized_value: Option<String>,
    value_type: String,
    state: String,
    evidence_ids: Vec<Uuid>,
    extractor_version: String,
    dossier_version: i64,
}

pub struct DecisionReceipt {
    pub field: Field,
    pub dossier_version: i64,
}

pub struct FieldDecisionCommand<'a> {
    pub dossier: Uuid,
    pub field: Uuid,
    pub value: &'a str,
    pub reason: &'a str,
    pub evidence: &'a [Uuid],
    pub decision: Uuid,
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
fn map_database(error: sqlx::Error) -> Error {
    match error
        .as_database_error()
        .and_then(|value| value.code())
        .as_deref()
    {
        Some("42501") => Error::Forbidden,
        Some("23514") | Some("23505") => Error::Conflict,
        _ => Error::Database(error),
    }
}
fn receipt(row: ReceiptRow) -> DecisionReceipt {
    DecisionReceipt {
        field: Field {
            id: row.id,
            path: row.path,
            raw_value: row.raw_value,
            normalized_value: row.normalized_value,
            value_type: row.value_type,
            state: row.state,
            evidence_ids: row.evidence_ids,
            extractor_version: row.extractor_version,
        },
        dossier_version: row.dossier_version,
    }
}
fn valid_value(value_type: &str, value: &str) -> bool {
    let kind = match value_type {
        "date" => FieldType::Date,
        "money" | "decimal" | "quantity" => FieldType::Decimal {
            scale: 8,
            signed: false,
        },
        "text" => FieldType::Text { max_bytes: 512 },
        _ => return false,
    };
    validate(kind, Some(value), true, true) == Outcome::Pass
}
fn request_hash(
    dossier: Uuid,
    field: Uuid,
    version: i64,
    value: &str,
    reason: &str,
    evidence: &[Uuid],
) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"finance.field.decide/v1");
    hash.update(dossier.as_bytes());
    hash.update(field.as_bytes());
    hash.update(version.to_be_bytes());
    for bytes in [value.as_bytes(), reason.as_bytes()] {
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
    }
    for id in evidence {
        hash.update(id.as_bytes());
    }
    hash.finalize().to_vec()
}

pub async fn decide(
    pool: &PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    command: FieldDecisionCommand<'_>,
) -> Result<DecisionReceipt, Error> {
    let FieldDecisionCommand {
        dossier,
        field,
        value,
        reason,
        evidence,
        decision,
    } = command;
    let Some(expected) = proof.context().expected_version else {
        return Err(Error::InvalidInput);
    };
    let version = i64::from(expected);
    if dossier.is_nil()
        || field.is_nil()
        || decision.is_nil()
        || value.trim() != value
        || value.is_empty()
        || value.len() > 512
        || value.chars().any(char::is_control)
        || reason.trim().is_empty()
        || reason.len() > 2048
        || evidence.len() > 64
        || evidence.iter().any(Uuid::is_nil)
        || evidence.iter().copied().collect::<BTreeSet<_>>().len() != evidence.len()
    {
        return Err(Error::InvalidInput);
    }
    let (tenant, company, actor) = binding(proof)?;
    let digest = request_hash(dossier, field, version, value, reason, evidence);
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Accountant)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!(
            "{tenant}/{company}/{actor}/field.decide/{}",
            key.as_str()
        ))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='field.decide' AND key=$4",
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
        let row = sqlx::query_as::<_, ReceiptRow>(
            "SELECT * FROM finance.runtime_field_receipt($1,$2,$3,$4,$5)",
        )
        .bind(tenant)
        .bind(company)
        .bind(dossier)
        .bind(actor)
        .bind(result)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_database)?
        .ok_or(Error::Conflict)?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(receipt(row));
    }
    let meta: Option<(String, String, Option<String>)> =
        sqlx::query_as("SELECT * FROM finance.runtime_field_meta($1,$2,$3,$4,$5)")
            .bind(tenant)
            .bind(company)
            .bind(dossier)
            .bind(actor)
            .bind(field)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_database)?;
    let Some((_path, value_type, _current)) = meta else {
        return Err(Error::Conflict);
    };
    if !valid_value(&value_type, value) {
        return Err(Error::InvalidInput);
    }
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    let new_version = sqlx::query_scalar::<_, i64>(
        "SELECT finance.runtime_field_decide($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
    )
    .bind(tenant)
    .bind(company)
    .bind(dossier)
    .bind(version)
    .bind(field)
    .bind(actor)
    .bind(decision)
    .bind(value)
    .bind(reason)
    .bind(evidence)
    .bind(command)
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_database)?;
    sqlx::query(
        "INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'field.decide',$4,$5,$6)",
    )
    .bind(tenant)
    .bind(company)
    .bind(actor)
    .bind(key.as_str())
    .bind(digest)
    .bind(decision)
    .execute(&mut *tx)
    .await?;
    let row = sqlx::query_as::<_, ReceiptRow>(
        "SELECT * FROM finance.runtime_field_receipt($1,$2,$3,$4,$5)",
    )
    .bind(tenant)
    .bind(company)
    .bind(dossier)
    .bind(actor)
    .bind(decision)
    .fetch_one(&mut *tx)
    .await
    .map_err(map_database)?;
    if row.dossier_version != new_version {
        return Err(Error::Conflict);
    }
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(receipt(row))
}
