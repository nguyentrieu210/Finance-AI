//! E02/E03 scoped membership administration.
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
    ids::{CompanyScope, ResourceScope},
};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, sqlx::FromRow)]
pub struct Membership {
    pub id: Uuid,
    pub subject_id: Uuid,
    pub company_id: Uuid,
    pub roles: Vec<String>,
    pub active: bool,
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

fn canonical_roles(roles: &[String]) -> Result<Vec<String>, Error> {
    if roles.is_empty() || roles.len() > 6 {
        return Err(Error::InvalidInput);
    }
    let input_len = roles.len();
    let mut roles = roles.to_vec();
    if roles.iter().any(|role| {
        !matches!(
            role.as_str(),
            "ORG_ADMIN"
                | "REQUESTER"
                | "ACCOUNTANT"
                | "AUDITOR"
                | "FINANCE_APPROVER"
                | "POLICY_ADMIN"
        )
    }) {
        return Err(Error::InvalidInput);
    }
    roles.sort();
    roles.dedup();
    if roles.len() != input_len {
        return Err(Error::InvalidInput);
    }
    Ok(roles)
}

fn roles_bytes(roles: &[String]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for role in roles {
        bytes.extend_from_slice(&(role.len() as u64).to_be_bytes());
        bytes.extend_from_slice(role.as_bytes());
    }
    bytes
}

fn request_hash(parts: &[&[u8]]) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"finance.membership.command/v1");
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.finalize().to_vec()
}

async fn read_one(
    tx: &mut Transaction<'_, Postgres>,
    t: Uuid,
    c: Uuid,
    a: Uuid,
    id: Uuid,
) -> Result<Option<Membership>, Error> {
    Ok(sqlx::query_as::<_, Membership>(
        "SELECT * FROM finance.runtime_memberships($1,$2,$3,$4,$5,$6)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(Uuid::nil())
    .bind(Some(id))
    .bind(1_i32)
    .fetch_optional(&mut **tx)
    .await?)
}

pub async fn list(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Membership>, Error> {
    if !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::OrgAdmin)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows = sqlx::query_as::<_, Membership>(
        "SELECT * FROM finance.runtime_memberships($1,$2,$3,$4,$5,$6)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(after)
    .bind(Option::<Uuid>::None)
    .bind(i32::try_from(limit).map_err(|_| Error::InvalidInput)?)
    .fetch_all(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn get(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    id: Uuid,
) -> Result<Option<Membership>, Error> {
    if id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::OrgAdmin)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = read_one(&mut tx, t, c, a, id).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn create(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    subject: Uuid,
    company: Uuid,
    roles: &[String],
) -> Result<Membership, Error> {
    if subject.is_nil() || proof.context().expected_version.is_some() {
        return Err(Error::InvalidInput);
    }
    let roles = canonical_roles(roles)?;
    let role_bytes = roles_bytes(&roles);
    let (t, c, a) = binding(proof)?;
    if company != c || subject == a {
        return Err(Error::Forbidden);
    }
    let hash = request_hash(&[subject.as_bytes(), company.as_bytes(), &role_bytes]);
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::OrgAdmin)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/membership.create/{}", key.as_str()))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record
         WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3
         AND operation_id='membership.create' AND key=$4",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(key.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((old, result)) = prior {
        if old != hash || result != subject {
            return Err(Error::Conflict);
        }
        let row = read_one(&mut tx, t, c, a, subject)
            .await?
            .ok_or(Error::Conflict)?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(row);
    }
    let version: i64 = sqlx::query_scalar("SELECT finance.membership_create($1,$2,$3,$4,$5)")
        .bind(t)
        .bind(c)
        .bind(a)
        .bind(subject)
        .bind(&roles)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_guard)?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query(
        "INSERT INTO finance.audit_event(
           tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action
         ) VALUES($1,$2,$3,$4,$3,$5,$6,'MEMBERSHIP_CREATED')",
    )
    .bind(t)
    .bind(c)
    .bind(command)
    .bind(a)
    .bind(correlation)
    .bind(subject)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
         VALUES($1,$2,$3,$3,$4,'MEMBERSHIP_CREATED')",
    )
    .bind(t)
    .bind(c)
    .bind(command)
    .bind(subject)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO finance.idempotency_record(
           tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_version
         ) VALUES($1,$2,$3,'membership.create',$4,$5,$6,$7)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(key.as_str())
    .bind(hash)
    .bind(subject)
    .bind(version)
    .execute(&mut *tx)
    .await?;
    let row = read_one(&mut tx, t, c, a, subject)
        .await?
        .ok_or(Error::Conflict)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

pub async fn revoke(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    subject: Uuid,
    reason: &str,
) -> Result<Membership, Error> {
    let Some(expected) = proof.context().expected_version else {
        return Err(Error::InvalidInput);
    };
    if subject.is_nil()
        || reason.trim().is_empty()
        || reason.len() > 2048
        || reason.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput);
    }
    let expected = i64::from(expected);
    let (t, c, a) = binding(proof)?;
    if subject == a {
        return Err(Error::Forbidden);
    }
    let hash = request_hash(&[
        subject.as_bytes(),
        &expected.to_be_bytes(),
        reason.as_bytes(),
    ]);
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::OrgAdmin)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/membership.revoke/{}", key.as_str()))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record
         WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3
         AND operation_id='membership.revoke' AND key=$4",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(key.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((old, result)) = prior {
        if old != hash || result != subject {
            return Err(Error::Conflict);
        }
        let row = read_one(&mut tx, t, c, a, subject)
            .await?
            .ok_or(Error::Conflict)?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(row);
    }
    let version: i64 = sqlx::query_scalar("SELECT finance.membership_revoke($1,$2,$3,$4,$5)")
        .bind(t)
        .bind(c)
        .bind(a)
        .bind(subject)
        .bind(expected)
        .fetch_one(&mut *tx)
        .await
        .map_err(map_guard)?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query(
        "INSERT INTO finance.audit_event(
           tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action
         ) VALUES($1,$2,$3,$4,$3,$5,$6,'MEMBERSHIP_REVOKED')",
    )
    .bind(t)
    .bind(c)
    .bind(command)
    .bind(a)
    .bind(correlation)
    .bind(subject)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type)
         VALUES($1,$2,$3,$3,$4,'MEMBERSHIP_REVOKED')",
    )
    .bind(t)
    .bind(c)
    .bind(command)
    .bind(subject)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO finance.idempotency_record(
           tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_version
         ) VALUES($1,$2,$3,'membership.revoke',$4,$5,$6,$7)",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(key.as_str())
    .bind(hash)
    .bind(subject)
    .bind(version)
    .execute(&mut *tx)
    .await?;
    let row = read_one(&mut tx, t, c, a, subject)
        .await?
        .ok_or(Error::Conflict)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

fn map_guard(error: sqlx::Error) -> Error {
    match &error {
        sqlx::Error::Database(database)
            if database.code().as_deref() == Some("23514")
                || database.code().as_deref() == Some("23503")
                || database.code().as_deref() == Some("23505") =>
        {
            Error::Conflict
        }
        sqlx::Error::Database(database) if database.code().as_deref() == Some("42501") => {
            Error::Forbidden
        }
        _ => Error::Database(error),
    }
}
