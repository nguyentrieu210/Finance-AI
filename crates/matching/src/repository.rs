//! E10 authenticated manual match confirmation and current-revision reads.
use finance_core::prototype::{Error,recheck_authenticated,scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError,Role,lock_roles,require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
    ids::{CompanyScope,ResourceScope},
};
use serde::Serialize;
use sha2::{Digest,Sha256};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone,Debug,Eq,PartialEq,Serialize,sqlx::FromRow)]
pub struct Match {
    pub id:Uuid,
    pub left_evidence_id:Uuid,
    pub right_evidence_id:Uuid,
    pub state:String,
    pub reason:String,
}
#[derive(sqlx::FromRow)]
struct Receipt {
    id:Uuid,left_evidence_id:Uuid,right_evidence_id:Uuid,state:String,reason:String,dossier_version:i64,
}
pub struct MatchReceipt { pub item:Match, pub dossier_version:i64 }

fn binding(proof:&AuthenticatedCommand)->Result<(Uuid,Uuid,Uuid),Error>{
    let ResourceScope::Company(CompanyScope{tenant,company})=proof.context().scope else{return Err(Error::Forbidden)};
    let Principal::User(actor)=proof.context().actor else{return Err(Error::Forbidden)};
    Ok((tenant.into(),company.into(),actor.into()))
}
fn grant(error:GrantError)->Error{match error{GrantError::Forbidden=>Error::Forbidden,GrantError::Database(e)=>Error::Database(e)}}
fn db(error:sqlx::Error)->Error{
    match error.as_database_error().and_then(|e|e.code()).as_deref(){
        Some("42501")=>Error::Forbidden,
        Some("23514")|Some("23505")=>Error::Conflict,
        _=>Error::Database(error),
    }
}
async fn reader(tx:&mut sqlx::Transaction<'_,sqlx::Postgres>,proof:&AuthenticatedCommand)->Result<(),Error>{
    let roles=lock_roles(tx,proof.context()).await.map_err(grant)?;
    if roles.contains(&Role::Requester)||roles.contains(&Role::Accountant){Ok(())}else{Err(Error::Forbidden)}
}
fn digest(dossier:Uuid,version:i64,left:Uuid,right:Uuid,reason:&str)->Vec<u8>{
    let mut h=Sha256::new();h.update(b"finance.match.confirm/v1");h.update(dossier.as_bytes());
    h.update(version.to_be_bytes());h.update(left.as_bytes());h.update(right.as_bytes());
    h.update((reason.len() as u64).to_be_bytes());h.update(reason.as_bytes());h.finalize().to_vec()
}
fn receipt(row:Receipt)->MatchReceipt{
    MatchReceipt{item:Match{id:row.id,left_evidence_id:row.left_evidence_id,right_evidence_id:row.right_evidence_id,state:row.state,reason:row.reason},dossier_version:row.dossier_version}
}
pub async fn list(pool:&PgPool,proof:&AuthenticatedCommand,dossier:Uuid,after:Uuid,limit:i64)->Result<Vec<Match>,Error>{
    if dossier.is_nil()||!(1..=100).contains(&limit){return Err(Error::InvalidInput)}
    let(t,c,a)=binding(proof)?;let mut tx=scoped_transaction(pool,proof.context()).await?;
    reader(&mut tx,proof).await?;recheck_authenticated(&mut tx,Some(proof)).await?;
    let rows=sqlx::query_as::<_,Match>("SELECT * FROM finance.runtime_matches($1,$2,$3,$4,$5,$6)")
        .bind(t).bind(c).bind(dossier).bind(a).bind(after).bind(limit).fetch_all(&mut *tx).await.map_err(db)?;
    recheck_authenticated(&mut tx,Some(proof)).await?;tx.commit().await?;Ok(rows)
}
pub async fn confirm(
    pool:&PgPool,proof:&AuthenticatedCommand,key:&IdempotencyKey,dossier:Uuid,left:Uuid,right:Uuid,
    reason:&str,result_id:Uuid,decision_id:Uuid
)->Result<MatchReceipt,Error>{
    let Some(expected)=proof.context().expected_version else{return Err(Error::InvalidInput)};
    let version=i64::from(expected);
    if dossier.is_nil()||left.is_nil()||right.is_nil()||left==right||result_id.is_nil()||decision_id.is_nil()
       ||reason.trim().is_empty()||reason.len()>2048||reason.chars().any(char::is_control){return Err(Error::InvalidInput)}
    let(t,c,a)=binding(proof)?;let hash=digest(dossier,version,left,right,reason);
    let mut tx=scoped_transaction(pool,proof.context()).await?;
    require_role(&mut tx,proof.context(),Role::Accountant).await.map_err(grant)?;
    recheck_authenticated(&mut tx,Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
      .bind(format!("{t}/{c}/{a}/match.confirm/{}",key.as_str())).execute(&mut *tx).await?;
    let prior:Option<(Vec<u8>,Uuid)>=sqlx::query_as(
      "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='match.confirm' AND key=$4")
      .bind(t).bind(c).bind(a).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((old,id))=prior{
        if old!=hash{return Err(Error::Conflict)}
        let row=sqlx::query_as::<_,Receipt>("SELECT * FROM finance.runtime_match_receipt($1,$2,$3,$4,$5)")
          .bind(t).bind(c).bind(dossier).bind(a).bind(id).fetch_optional(&mut *tx).await.map_err(db)?.ok_or(Error::Conflict)?;
        recheck_authenticated(&mut tx,Some(proof)).await?;tx.commit().await?;return Ok(receipt(row))
    }
    let command:Uuid=proof.context().command.into();let correlation:Uuid=proof.context().correlation.into();
    let new_version=sqlx::query_scalar::<_,i64>(
      "SELECT finance.runtime_match_confirm($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)")
      .bind(t).bind(c).bind(dossier).bind(version).bind(a).bind(result_id).bind(decision_id)
      .bind(left).bind(right).bind(reason).bind(command).bind(correlation)
      .fetch_one(&mut *tx).await.map_err(db)?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'match.confirm',$4,$5,$6)")
      .bind(t).bind(c).bind(a).bind(key.as_str()).bind(hash).bind(result_id).execute(&mut *tx).await?;
    let row=sqlx::query_as::<_,Receipt>("SELECT * FROM finance.runtime_match_receipt($1,$2,$3,$4,$5)")
      .bind(t).bind(c).bind(dossier).bind(a).bind(result_id).fetch_one(&mut *tx).await.map_err(db)?;
    if row.dossier_version!=new_version{return Err(Error::Conflict)}
    recheck_authenticated(&mut tx,Some(proof)).await?;tx.commit().await?;Ok(receipt(row))
}
