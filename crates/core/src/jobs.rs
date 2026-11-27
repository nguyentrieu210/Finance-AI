//! Synthetic-only durable outbox consumer. No external effects or live worker loop.
use finance_shared::ids::CompanyScope;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug)]
pub enum JobError {
    Forbidden,
    InvalidInput,
    StaleLease,
    Invariant,
    Database(sqlx::Error),
}
impl From<sqlx::Error> for JobError {
    fn from(e: sqlx::Error) -> Self {
        Self::Database(e)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease {
    scope: CompanyScope,
    pub job_id: Uuid,
    pub fence: i64,
    worker: Uuid,
}
impl Lease {
    pub fn scope(self) -> CompanyScope {
        self.scope
    }

    pub fn worker_id(self) -> Uuid {
        self.worker
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Transient,
    Permanent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureResult {
    Retry,
    Dead,
}

fn ids(scope: CompanyScope) -> (Uuid, Uuid) {
    (scope.tenant.into(), scope.company.into())
}
async fn begin(
    pool: &PgPool,
    scope: CompanyScope,
) -> Result<Transaction<'static, Postgres>, JobError> {
    let mut tx = pool.begin().await?;
    let role:bool=sqlx::query_scalar("SELECT current_user='finance_job_worker' AND NOT rolsuper AND NOT rolbypassrls AND NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=session_user AND (rolsuper OR rolbypassrls OR rolcreaterole)) FROM pg_roles WHERE rolname=current_user").fetch_one(&mut *tx).await?;
    if !role {
        return Err(JobError::Forbidden);
    }
    let (t, c) = ids(scope);
    sqlx::query(
        "SELECT set_config('finance.tenant_id',$1,true),set_config('finance.company_id',$2,true)",
    )
    .bind(t.to_string())
    .bind(c.to_string())
    .execute(&mut *tx)
    .await?;
    let allowed: bool = sqlx::query_scalar("SELECT finance.worker_scope_allowed($1,$2)")
        .bind(t)
        .bind(c)
        .fetch_one(&mut *tx)
        .await?;
    if !allowed {
        return Err(JobError::Forbidden);
    }
    Ok(tx)
}
fn valid_lease(seconds: i32) -> Result<(), JobError> {
    if !(1..=300).contains(&seconds) {
        Err(JobError::InvalidInput)
    } else {
        Ok(())
    }
}
/// Materialize a bounded batch from immutable transactional outbox. Unique ID makes concurrent replay safe.
pub async fn dispatch(pool: &PgPool, scope: CompanyScope, limit: i64) -> Result<u64, JobError> {
    if !(1..=100).contains(&limit) {
        return Err(JobError::InvalidInput);
    }
    let mut tx = begin(pool, scope).await?;
    let (t, c) = ids(scope);
    let n=sqlx::query("INSERT INTO finance.job(tenant_id,company_id,id) SELECT o.tenant_id,o.company_id,o.id FROM finance.outbox o WHERE o.tenant_id=$1 AND o.company_id=$2 AND NOT EXISTS(SELECT 1 FROM finance.job j WHERE (j.tenant_id,j.company_id,j.id)=(o.tenant_id,o.company_id,o.id)) ORDER BY o.created_at,o.id LIMIT $3 ON CONFLICT DO NOTHING")
        .bind(t).bind(c).bind(limit).execute(&mut *tx).await?.rows_affected();
    tx.commit().await?;
    Ok(n)
}
async fn finish_attempt(
    tx: &mut Transaction<'_, Postgres>,
    t: Uuid,
    c: Uuid,
    id: Uuid,
    fence: i64,
    state: &str,
) -> Result<(), JobError> {
    let n=sqlx::query("UPDATE finance.job_attempt SET state=$5,finished_at=clock_timestamp() WHERE tenant_id=$1 AND company_id=$2 AND job_id=$3 AND fence=$4 AND state='ACTIVE'")
        .bind(t).bind(c).bind(id).bind(fence).bind(state).execute(&mut **tx).await?.rows_affected();
    if n != 1 {
        return Err(JobError::Invariant);
    }
    Ok(())
}
/// Lock one eligible job, recover expired lease and increment fencing token. Dead jobs are not reclaimed.
pub async fn claim(
    pool: &PgPool,
    scope: CompanyScope,
    worker: Uuid,
    seconds: i32,
) -> Result<Option<Lease>, JobError> {
    valid_lease(seconds)?;
    if worker.is_nil() {
        return Err(JobError::InvalidInput);
    }
    let mut tx = begin(pool, scope).await?;
    let (t, c) = ids(scope);
    let row:Option<(Uuid,String,i32,i64)>=sqlx::query_as("SELECT id,state,attempts,fence FROM finance.job WHERE tenant_id=$1 AND company_id=$2 AND ((state IN ('QUEUED','RETRY') AND available_at<=clock_timestamp()) OR (state='LEASED' AND lease_until<=clock_timestamp())) ORDER BY available_at,id FOR UPDATE SKIP LOCKED LIMIT 1")
        .bind(t).bind(c).fetch_optional(&mut *tx).await?;
    let Some((id, state, attempts, fence)) = row else {
        tx.commit().await?;
        return Ok(None);
    };
    if state == "LEASED" {
        finish_attempt(&mut tx, t, c, id, fence, "EXPIRED").await?;
    }
    if attempts >= 3 {
        sqlx::query("UPDATE finance.job SET state='DEAD',lease_until=NULL,leased_by=NULL,last_error='LEASE_EXPIRED' WHERE tenant_id=$1 AND company_id=$2 AND id=$3").bind(t).bind(c).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        return Ok(None);
    }
    let new_fence:i64=sqlx::query_scalar("UPDATE finance.job SET state='LEASED',attempts=attempts+1,fence=fence+1,leased_by=$4,lease_until=clock_timestamp()+make_interval(secs=>$5::double precision),last_error=NULL WHERE tenant_id=$1 AND company_id=$2 AND id=$3 RETURNING fence")
        .bind(t).bind(c).bind(id).bind(worker).bind(seconds).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.job_attempt(tenant_id,company_id,job_id,fence,worker_id,state,lease_until) SELECT tenant_id,company_id,id,fence,leased_by,'ACTIVE',lease_until FROM finance.job WHERE tenant_id=$1 AND company_id=$2 AND id=$3")
        .bind(t).bind(c).bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Some(Lease {
        scope,
        job_id: id,
        fence: new_fence,
        worker,
    }))
}
pub async fn heartbeat(pool: &PgPool, lease: Lease, seconds: i32) -> Result<(), JobError> {
    valid_lease(seconds)?;
    let mut tx = begin(pool, lease.scope).await?;
    let (t, c) = ids(lease.scope);
    let n=sqlx::query("UPDATE finance.job SET lease_until=GREATEST(lease_until,clock_timestamp()+make_interval(secs=>$6::double precision)) WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND fence=$4 AND leased_by=$5 AND state='LEASED' AND lease_until>clock_timestamp()")
        .bind(t).bind(c).bind(lease.job_id).bind(lease.fence).bind(lease.worker).bind(seconds).execute(&mut *tx).await?.rows_affected();
    if n != 1 {
        return Err(JobError::StaleLease);
    }
    let n=sqlx::query("UPDATE finance.job_attempt a SET lease_until=j.lease_until FROM finance.job j WHERE (a.tenant_id,a.company_id,a.job_id,a.fence)=(j.tenant_id,j.company_id,j.id,j.fence) AND j.tenant_id=$1 AND j.company_id=$2 AND j.id=$3 AND a.state='ACTIVE'")
        .bind(t).bind(c).bind(lease.job_id).execute(&mut *tx).await?.rows_affected();
    if n != 1 {
        return Err(JobError::Invariant);
    }
    tx.commit().await?;
    Ok(())
}
/// Synthetic handler: durable inbox receipt is the only effect. Commit with the fenced job state.
pub async fn complete(pool: &PgPool, lease: Lease) -> Result<(), JobError> {
    let mut tx = begin(pool, lease.scope).await?;
    let (t, c) = ids(lease.scope);
    let n=sqlx::query("UPDATE finance.job SET state='SUCCEEDED',lease_until=NULL,leased_by=NULL WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND fence=$4 AND leased_by=$5 AND state='LEASED' AND lease_until>clock_timestamp()")
        .bind(t).bind(c).bind(lease.job_id).bind(lease.fence).bind(lease.worker).execute(&mut *tx).await?.rows_affected();
    if n != 1 {
        return Err(JobError::StaleLease);
    }
    finish_attempt(&mut tx, t, c, lease.job_id, lease.fence, "SUCCEEDED").await?;
    sqlx::query(
        "INSERT INTO finance.inbox(tenant_id,company_id,event_id,fence) VALUES($1,$2,$3,$4)",
    )
    .bind(t)
    .bind(c)
    .bind(lease.job_id)
    .bind(lease.fence)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn fail(
    pool: &PgPool,
    lease: Lease,
    failure: Failure,
) -> Result<FailureResult, JobError> {
    let mut tx = begin(pool, lease.scope).await?;
    let (t, c) = ids(lease.scope);
    let permanent = failure == Failure::Permanent;
    let code = if permanent { "PERMANENT" } else { "TRANSIENT" };
    let state:Option<String>=sqlx::query_scalar("UPDATE finance.job SET state=CASE WHEN $6 OR attempts>=3 THEN 'DEAD' ELSE 'RETRY' END,available_at=clock_timestamp()+make_interval(secs=>attempts::double precision),lease_until=NULL,leased_by=NULL,last_error=$7 WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND fence=$4 AND leased_by=$5 AND state='LEASED' AND lease_until>clock_timestamp() RETURNING state")
        .bind(t).bind(c).bind(lease.job_id).bind(lease.fence).bind(lease.worker).bind(permanent).bind(code).fetch_optional(&mut *tx).await?;
    let state = state.ok_or(JobError::StaleLease)?;
    finish_attempt(&mut tx, t, c, lease.job_id, lease.fence, &state).await?;
    tx.commit().await?;
    Ok(if state == "DEAD" {
        FailureResult::Dead
    } else {
        FailureResult::Retry
    })
}
