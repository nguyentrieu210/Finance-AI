#![forbid(unsafe_code)]
//! Authenticated organization and membership commands.
pub mod memberships;
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_permissions::{GrantError, Role, require_role};
use finance_shared::{
    contracts::{CommandContext, Principal, Version},
    http_contract::IdempotencyKey,
    ids::ResourceScope,
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub id: Uuid,
    pub parent: Option<Uuid>,
    pub code: String,
    pub name: String,
    pub kind: String,
    pub active: bool,
    pub version: i64,
}
pub struct UpdateUnit<'a> {
    pub id: Uuid,
    pub parent: Option<Uuid>,
    pub name: &'a str,
    pub active: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Receipt {
    pub id: Uuid,
    pub version: Version,
    pub replayed: bool,
}
fn hash(input: &UpdateUnit<'_>, version: Version) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(b"prototype.updateOrganizationUnit/v1");
    h.update(input.id.as_bytes());
    match input.parent {
        Some(p) => {
            h.update([1]);
            h.update(p.as_bytes());
        }
        None => h.update([0]),
    }
    h.update(i64::from(version).to_be_bytes());
    h.update([u8::from(input.active)]);
    h.update((input.name.len() as u64).to_be_bytes());
    h.update(input.name.as_bytes());
    h.finalize().to_vec()
}
fn map_grant(e: GrantError) -> Error {
    match e {
        GrantError::Forbidden => Error::Forbidden,
        GrantError::Database(e) => Error::Database(e),
    }
}
fn mutation_error(e: sqlx::Error) -> Error {
    if e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23514" || c == "23503" || c == "23505")
    {
        Error::Conflict
    } else {
        Error::Database(e)
    }
}
pub async fn update_unit(
    pool: &PgPool,
    ctx: &CommandContext,
    key: &IdempotencyKey,
    input: &UpdateUnit<'_>,
) -> Result<Receipt, Error> {
    update_unit_inner(pool, ctx, key, input, None).await
}
pub async fn update_unit_authenticated(
    pool: &PgPool,
    proof: &finance_identity::AuthenticatedCommand,
    key: &IdempotencyKey,
    input: &UpdateUnit<'_>,
) -> Result<Receipt, Error> {
    update_unit_inner(pool, proof.context(), key, input, Some(proof)).await
}
async fn update_unit_inner(
    pool: &PgPool,
    ctx: &CommandContext,
    key: &IdempotencyKey,
    input: &UpdateUnit<'_>,
    proof: Option<&finance_identity::AuthenticatedCommand>,
) -> Result<Receipt, Error> {
    let expected = ctx.expected_version.ok_or(Error::InvalidInput)?;
    if input.id.is_nil()
        || input.parent.is_some_and(|p| p.is_nil() || p == input.id)
        || input.name.trim().is_empty()
        || input.name.chars().count() > 200
        || i64::from(expected) == 9_007_199_254_740_991
    {
        return Err(Error::InvalidInput);
    }
    let ResourceScope::Company(scope) = ctx.scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(actor) = ctx.actor else {
        return Err(Error::Forbidden);
    };
    let (t, c, a): (Uuid, Uuid, Uuid) = (scope.tenant.into(), scope.company.into(), actor.into());
    let cmd: Uuid = ctx.command.into();
    let corr: Uuid = ctx.correlation.into();
    let mut tx = scoped_transaction(pool, ctx).await?;
    let lock = format!(
        "{t}/{c}/{a}/prototype.updateOrganizationUnit/{}",
        key.as_str()
    );
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(lock)
        .execute(&mut *tx)
        .await?;
    require_role(&mut tx, ctx, Role::OrgAdmin)
        .await
        .map_err(map_grant)?;
    recheck_authenticated(&mut tx, proof).await?;
    let h = hash(input, expected);
    let prior:Option<(Vec<u8>,Uuid,i64)>=sqlx::query_as("SELECT request_hash,result_id,result_version FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='prototype.updateOrganizationUnit' AND key=$4").bind(t).bind(c).bind(a).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((old, id, v)) = prior {
        if old != h {
            return Err(Error::Conflict);
        }
        let version = Version::try_from(v).map_err(|_| Error::Conflict)?;
        recheck_authenticated(&mut tx, proof).await?;
        tx.commit().await?;
        return Ok(Receipt {
            id,
            version,
            replayed: true,
        });
    }
    // Acquire company tree lock before UPDATE starts; trigger also guards direct scoped writes.
    sqlx::query("SELECT finance.lock_org_tree($1,$2)")
        .bind(t)
        .bind(c)
        .execute(&mut *tx)
        .await?;
    recheck_authenticated(&mut tx, proof).await?;
    let v:Option<i64>=sqlx::query_scalar("UPDATE finance.organization_unit SET parent_id=$4,name=$5,active=$6,version=version+1 WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND version=$7 RETURNING version")
  .bind(t).bind(c).bind(input.id).bind(input.parent).bind(input.name).bind(input.active).bind(i64::from(expected)).fetch_optional(&mut *tx).await.map_err(mutation_error)?;
    let v = v.ok_or(Error::Conflict)?;
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'ORG_UNIT_UPDATED')").bind(t).bind(c).bind(cmd).bind(a).bind(corr).bind(input.id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'ORG_UNIT_UPDATED')").bind(t).bind(c).bind(cmd).bind(input.id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_version) VALUES($1,$2,$3,'prototype.updateOrganizationUnit',$4,$5,$6,$7)").bind(t).bind(c).bind(a).bind(key.as_str()).bind(h).bind(input.id).bind(v).execute(&mut *tx).await?;
    recheck_authenticated(&mut tx, proof).await?;
    tx.commit().await?;
    Ok(Receipt {
        id: input.id,
        version: Version::try_from(v).map_err(|_| Error::Conflict)?,
        replayed: false,
    })
}
/// Scoped admin page, not unrestricted company listing. Inactive units remain visible for history.
pub async fn list_units(
    pool: &PgPool,
    ctx: &CommandContext,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Unit>, Error> {
    list_units_inner(pool, ctx, after, limit, None).await
}
pub async fn list_units_authenticated(
    pool: &PgPool,
    proof: &finance_identity::AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Unit>, Error> {
    list_units_inner(pool, proof.context(), after, limit, Some(proof)).await
}
async fn list_units_inner(
    pool: &PgPool,
    ctx: &CommandContext,
    after: Uuid,
    limit: i64,
    proof: Option<&finance_identity::AuthenticatedCommand>,
) -> Result<Vec<Unit>, Error> {
    if !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let ResourceScope::Company(scope) = ctx.scope else {
        return Err(Error::Forbidden);
    };
    let (t, c): (Uuid, Uuid) = (scope.tenant.into(), scope.company.into());
    let mut tx = scoped_transaction(pool, ctx).await?;
    require_role(&mut tx, ctx, Role::OrgAdmin)
        .await
        .map_err(map_grant)?;
    recheck_authenticated(&mut tx, proof).await?;
    type UnitRow = (Uuid, Option<Uuid>, String, String, String, bool, i64);
    let rows:Vec<UnitRow>=sqlx::query_as("SELECT id,parent_id,code,name,kind,active,version FROM finance.organization_unit WHERE tenant_id=$1 AND company_id=$2 AND id>$3 ORDER BY id LIMIT $4").bind(t).bind(c).bind(after).bind(limit).fetch_all(&mut *tx).await?;
    recheck_authenticated(&mut tx, proof).await?;
    tx.commit().await?;
    Ok(rows
        .into_iter()
        .map(|(id, parent, code, name, kind, active, version)| Unit {
            id,
            parent,
            code,
            name,
            kind,
            active,
            version,
        })
        .collect())
}

pub async fn get_unit_authenticated(
    pool: &PgPool,
    proof: &finance_identity::AuthenticatedCommand,
    id: Uuid,
) -> Result<Option<Unit>, Error> {
    if id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let ctx = proof.context();
    let ResourceScope::Company(scope) = ctx.scope else {
        return Err(Error::Forbidden);
    };
    let (t, c): (Uuid, Uuid) = (scope.tenant.into(), scope.company.into());
    let mut tx = scoped_transaction(pool, ctx).await?;
    require_role(&mut tx, ctx, Role::OrgAdmin)
        .await
        .map_err(map_grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    type UnitRow = (Uuid, Option<Uuid>, String, String, String, bool, i64);
    let row: Option<UnitRow> = sqlx::query_as(
        "SELECT id,parent_id,code,name,kind,active,version
         FROM finance.organization_unit
         WHERE tenant_id=$1 AND company_id=$2 AND id=$3",
    )
    .bind(t)
    .bind(c)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(
        row.map(|(id, parent, code, name, kind, active, version)| Unit {
            id,
            parent,
            code,
            name,
            kind,
            active,
            version,
        }),
    )
}
