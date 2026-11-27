//! Isolated synthetic prototype. No HTTP routes and no untrusted caller may construct context.
use finance_shared::{
    contracts::{CommandContext, Principal},
    http_contract::IdempotencyKey,
    ids::{CompanyScope, ResourceScope},
};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug)]
pub enum Error {
    Forbidden,
    InvalidInput,
    Conflict,
    Database(sqlx::Error),
}
impl From<sqlx::Error> for Error {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Department,
    CostCenter,
}
impl UnitKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Department => "DEPARTMENT",
            Self::CostCenter => "COST_CENTER",
        }
    }
}
pub struct CreateUnit<'a> {
    pub id: Uuid,
    pub parent: Option<Uuid>,
    pub code: &'a str,
    pub name: &'a str,
    pub kind: UnitKind,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receipt {
    Created(Uuid),
    Replayed(Uuid),
}

pub async fn scoped_transaction(
    pool: &PgPool,
    context: &CommandContext,
) -> Result<Transaction<'static, Postgres>, Error> {
    let ResourceScope::Company(scope) = context.scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(actor) = context.actor else {
        return Err(Error::Forbidden);
    };
    let mut tx = pool.begin().await?;
    // Reject privileged connections, including migration accounts. Caller-provided GUCs are not trusted.
    let runtime: bool=sqlx::query_scalar("SELECT current_user = 'finance_runtime' AND NOT rolsuper AND NOT rolbypassrls AND NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname=session_user AND (rolsuper OR rolbypassrls OR rolcreaterole)) FROM pg_roles WHERE rolname=current_user").fetch_one(&mut *tx).await?;
    if !runtime {
        return Err(Error::Forbidden);
    }
    sqlx::query("SELECT set_config('finance.tenant_id',$1,true),set_config('finance.company_id',$2,true),set_config('finance.actor_id',$3,true)")
        .bind(scope.tenant.to_string()).bind(scope.company.to_string()).bind(actor.to_string()).execute(&mut *tx).await?;
    Ok(tx)
}
fn hash_command(input: &CreateUnit<'_>) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"prototype.createOrganizationUnit/v1");
    hash.update(input.id.as_bytes());
    match input.parent {
        Some(p) => {
            hash.update([1]);
            hash.update(p.as_bytes());
        }
        None => hash.update([0]),
    }
    for s in [input.code, input.name, input.kind.as_str()] {
        hash.update((s.len() as u64).to_be_bytes());
        hash.update(s.as_bytes());
    }
    hash.finalize().to_vec()
}
/// This is an internal prototype operation, not an OpenAPI handler. Trusted context only.
/// Role revocation races, hierarchy cycle updates, full ABAC, audit hash chain and production roles are later work.
pub async fn create_unit(
    pool: &PgPool,
    context: &CommandContext,
    key: &IdempotencyKey,
    input: &CreateUnit<'_>,
) -> Result<Receipt, Error> {
    if input.id.is_nil()
        || input.parent.is_some_and(|p| p.is_nil() || p == input.id)
        || input.code.trim() != input.code
        || input.code.is_empty()
        || input.code.chars().count() > 64
        || input.name.trim().is_empty()
        || input.name.chars().count() > 200
        || context.expected_version.is_some()
    {
        return Err(Error::InvalidInput);
    }
    let ResourceScope::Company(CompanyScope { tenant, company }) = context.scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(actor) = context.actor else {
        return Err(Error::Forbidden);
    };
    let (tenant, company, actor): (Uuid, Uuid, Uuid) =
        (tenant.into(), company.into(), actor.into());
    let command: Uuid = context.command.into();
    let correlation: Uuid = context.correlation.into();
    let mut tx = scoped_transaction(pool, context).await?;
    // Serialize retries of this scoped operation/key. Collisions only cause harmless contention.
    let lock = format!(
        "{tenant}/{company}/{actor}/prototype.createOrganizationUnit/{}",
        key.as_str()
    );
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(lock)
        .execute(&mut *tx)
        .await?;
    let allowed:bool=sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM finance.membership m JOIN finance.company c ON (c.tenant_id,c.id)=(m.tenant_id,m.company_id) JOIN finance.tenant t ON t.id=m.tenant_id JOIN finance.identity_subject i ON (i.tenant_id,i.id)=(m.tenant_id,m.actor_id) WHERE m.tenant_id=$1 AND m.company_id=$2 AND m.actor_id=$3 AND m.role='ORG_ADMIN' AND m.active AND c.active AND t.active AND i.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))")
        .bind(tenant).bind(company).bind(actor).fetch_one(&mut *tx).await?;
    if !allowed {
        return Err(Error::Forbidden);
    }
    let request_hash = hash_command(input);
    let prior:Option<(Vec<u8>,Uuid)>=sqlx::query_as("SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='prototype.createOrganizationUnit' AND key=$4")
        .bind(tenant).bind(company).bind(actor).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((hash, id)) = prior {
        if hash != request_hash {
            return Err(Error::Conflict);
        }
        tx.commit().await?;
        return Ok(Receipt::Replayed(id));
    }
    sqlx::query("INSERT INTO finance.organization_unit(tenant_id,company_id,id,parent_id,code,name,kind) VALUES($1,$2,$3,$4,$5,$6,$7)")
        .bind(tenant).bind(company).bind(input.id).bind(input.parent).bind(input.code).bind(input.name).bind(input.kind.as_str()).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'ORG_UNIT_CREATED')")
        .bind(tenant).bind(company).bind(command).bind(actor).bind(correlation).bind(input.id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'ORG_UNIT_CREATED')")
        .bind(tenant).bind(company).bind(command).bind(input.id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'prototype.createOrganizationUnit',$4,$5,$6)")
        .bind(tenant).bind(company).bind(actor).bind(key.as_str()).bind(request_hash).bind(input.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Receipt::Created(input.id))
}
