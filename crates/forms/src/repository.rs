//! E19 immutable form aggregate repository.
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, lock_roles, require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
    ids::{CompanyScope, ResourceScope},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftField {
    pub path: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_row: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_column: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_width: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_ref: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct Form {
    pub id: Uuid,
    pub policy_id: Uuid,
    pub version: i64,
    pub name: String,
    pub schema_hash: String,
    pub state: String,
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

fn schema(fields: &[DraftField], version: i64) -> Result<Value, Error> {
    if fields.is_empty() || fields.len() > 256 || !(1..=9_007_199_254_740_991).contains(&version) {
        return Err(Error::InvalidInput);
    }
    let mut known = BTreeMap::<String, String>::new();
    let mut normalized = Vec::with_capacity(fields.len());
    for (index, field) in fields.iter().enumerate() {
        if field.label.trim().is_empty()
            || field.label.len() > 200
            || field.label.chars().any(char::is_control)
            || !matches!(
                field.field_type.as_str(),
                "text" | "date" | "decimal" | "money" | "boolean" | "enum"
            )
        {
            return Err(Error::InvalidInput);
        }
        if field.field_type == "enum" {
            let Some(options) = field.options.as_ref() else {
                return Err(Error::InvalidInput);
            };
            if options.is_empty() || options.len() > 64 {
                return Err(Error::InvalidInput);
            }
        } else if field.options.is_some() {
            return Err(Error::InvalidInput);
        }
        if let Some(control) = field.condition_ref.as_ref() {
            if known.get(control).map(String::as_str) != Some("boolean") {
                return Err(Error::InvalidInput);
            }
        }
        if known
            .insert(field.path.clone(), field.field_type.clone())
            .is_some()
        {
            return Err(Error::InvalidInput);
        }
        let layout = match (field.layout_row, field.layout_column, field.layout_width) {
            (None, None, None) => json!({"row":index,"column":0,"width":12}),
            (Some(row), Some(column), Some(width)) => {
                json!({"row":row,"column":column,"width":width})
            }
            _ => return Err(Error::InvalidInput),
        };
        let source_path = field.source_path.as_deref().unwrap_or(&field.path);
        let visible = field
            .condition_ref
            .as_ref()
            .map(|control| json!({"field":control,"equals":true}));
        normalized.push(json!({
            "key": field.path,
            "type": field.field_type,
            "required": field.required && field.condition_ref.is_none(),
            "required_when_visible": field.required && field.condition_ref.is_some(),
            "visible_if": visible,
            "options": field.options,
            "source_path": source_path,
            "layout": layout
        }));
    }
    let value = json!({"version":version,"fields":normalized});
    let bytes = serde_json::to_vec(&value).map_err(|_| Error::InvalidInput)?;
    crate::schema::compile(&bytes).map_err(|_| Error::InvalidInput)?;
    Ok(value)
}

const SELECT_FORM: &str = "SELECT form_id AS id,policy_id,version,name,encode(sha256(convert_to(schema_json::text,'UTF8')),'hex') AS schema_hash,CASE WHEN state='REVOKED' THEN 'RETIRED' ELSE state END AS state FROM finance.form_version";

async fn allowed_read(
    tx: &mut Transaction<'_, Postgres>,
    proof: &AuthenticatedCommand,
) -> Result<(), Error> {
    let roles = lock_roles(tx, proof.context()).await.map_err(|e| match e {
        GrantError::Forbidden => Error::Forbidden,
        GrantError::Database(e) => Error::Database(e),
    })?;
    if roles.contains(&Role::Requester) || roles.contains(&Role::PolicyAdmin) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

fn request_hash(parts: &[&[u8]]) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"finance.form.command/v1");
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.finalize().to_vec()
}

async fn latest(
    tx: &mut Transaction<'_, Postgres>,
    t: Uuid,
    c: Uuid,
    form: Uuid,
) -> Result<Form, Error> {
    Ok(sqlx::query_as::<_, Form>(&format!(
        "{SELECT_FORM} WHERE tenant_id=$1 AND company_id=$2 AND form_id=$3 ORDER BY version DESC LIMIT 1"
    ))
    .bind(t)
    .bind(c)
    .bind(form)
    .fetch_one(&mut **tx)
    .await?)
}

pub async fn create(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    policy: Uuid,
    form: Uuid,
    row: Uuid,
    name: &str,
    fields: &[DraftField],
) -> Result<Form, Error> {
    if policy.is_nil()
        || form.is_nil()
        || row.is_nil()
        || form == row
        || name.trim().is_empty()
        || name.len() > 200
        || name.chars().any(char::is_control)
        || proof.context().expected_version.is_some()
    {
        return Err(Error::InvalidInput);
    }
    let schema = schema(fields, 1)?;
    let payload = schema.to_string();
    let hash = request_hash(&[policy.as_bytes(), name.as_bytes(), payload.as_bytes()]);
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::PolicyAdmin)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/form.create/{}", key.as_str()))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='form.create' AND key=$4",
    )
    .bind(t).bind(c).bind(a).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((old, result)) = prior {
        if old != hash {
            return Err(Error::Conflict);
        }
        let receipt = latest(&mut tx, t, c, result).await?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(receipt);
    }
    let policy_ok: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM finance.policy_version WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND author_id=$4 AND state='DRAFT')",
    )
    .bind(t).bind(c).bind(policy).bind(a).fetch_one(&mut *tx).await?;
    if !policy_ok {
        return Err(Error::Conflict);
    }
    sqlx::query(
        "INSERT INTO finance.form_version(tenant_id,company_id,id,form_id,policy_id,name,version,state,schema_json,author_id) VALUES($1,$2,$3,$4,$5,$6,1,'DRAFT',$7::text::jsonb,$8)",
    )
    .bind(t).bind(c).bind(row).bind(form).bind(policy).bind(name).bind(&payload).bind(a)
    .execute(&mut *tx).await?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'FORM_DRAFT_CREATED')")
        .bind(t).bind(c).bind(command).bind(a).bind(correlation).bind(form).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'FORM_DRAFT_CREATED')")
        .bind(t).bind(c).bind(command).bind(form).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'form.create',$4,$5,$6)")
        .bind(t).bind(c).bind(a).bind(key.as_str()).bind(hash).bind(form).execute(&mut *tx).await?;
    let receipt = latest(&mut tx, t, c, form).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(receipt)
}

pub async fn revise(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    form: Uuid,
    row: Uuid,
    name: &str,
    fields: &[DraftField],
    reason: &str,
) -> Result<Form, Error> {
    let Some(expected) = proof.context().expected_version else {
        return Err(Error::InvalidInput);
    };
    let expected = i64::from(expected);
    if form.is_nil()
        || row.is_nil()
        || name.trim().is_empty()
        || name.len() > 200
        || reason.trim().is_empty()
        || reason.len() > 2048
    {
        return Err(Error::InvalidInput);
    }
    let next = expected.checked_add(1).ok_or(Error::Conflict)?;
    let schema = schema(fields, next)?;
    let payload = schema.to_string();
    let hash = request_hash(&[
        form.as_bytes(),
        &expected.to_be_bytes(),
        name.as_bytes(),
        payload.as_bytes(),
        reason.as_bytes(),
    ]);
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::PolicyAdmin)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/form.revise/{}", key.as_str()))
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("finance.form/v1/{t}/{c}/{form}"))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='form.revise' AND key=$4",
    )
    .bind(t).bind(c).bind(a).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((old, result)) = prior {
        if old != hash || result != form {
            return Err(Error::Conflict);
        }
        let receipt = latest(&mut tx, t, c, form).await?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(receipt);
    }
    let current = latest(&mut tx, t, c, form).await?;
    if current.version != expected || current.state != "DRAFT" {
        return Err(Error::Conflict);
    }
    sqlx::query(
        "INSERT INTO finance.form_version(tenant_id,company_id,id,form_id,policy_id,name,version,state,schema_json,author_id) SELECT tenant_id,company_id,$4,form_id,policy_id,$5,$6,'DRAFT',$7::text::jsonb,author_id FROM finance.form_version WHERE tenant_id=$1 AND company_id=$2 AND form_id=$3 AND version=$8 AND author_id=$9",
    )
    .bind(t).bind(c).bind(form).bind(row).bind(name).bind(next).bind(&payload).bind(expected).bind(a)
    .execute(&mut *tx).await?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'FORM_DRAFT_REVISED')")
        .bind(t).bind(c).bind(command).bind(a).bind(correlation).bind(form).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'FORM_DRAFT_REVISED')")
        .bind(t).bind(c).bind(command).bind(form).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'form.revise',$4,$5,$6)")
        .bind(t).bind(c).bind(a).bind(key.as_str()).bind(hash).bind(form).execute(&mut *tx).await?;
    let receipt = latest(&mut tx, t, c, form).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(receipt)
}

pub async fn list(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Form>, Error> {
    if !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (t, c, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    allowed_read(&mut tx, proof).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows=sqlx::query_as::<_,Form>(&format!(
        "SELECT id,policy_id,version,name,schema_hash,state FROM (SELECT DISTINCT ON(form_id) form_id AS id,policy_id,version,name,encode(sha256(convert_to(schema_json::text,'UTF8')),'hex') AS schema_hash,CASE WHEN state='REVOKED' THEN 'RETIRED' ELSE state END AS state FROM finance.form_version WHERE tenant_id=$1 AND company_id=$2 AND form_id>$3 ORDER BY form_id,version DESC) f ORDER BY id LIMIT $4"
    )).bind(t).bind(c).bind(after).bind(limit).fetch_all(&mut *tx).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn get(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    form: Uuid,
) -> Result<Option<Form>, Error> {
    if form.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (t, c, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    allowed_read(&mut tx, proof).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row=sqlx::query_as::<_,Form>(&format!(
        "{SELECT_FORM} WHERE tenant_id=$1 AND company_id=$2 AND form_id=$3 ORDER BY version DESC LIMIT 1"
    )).bind(t).bind(c).bind(form).fetch_optional(&mut *tx).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
