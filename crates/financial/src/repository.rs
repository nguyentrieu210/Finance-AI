//! E11 scoped read projection. Mutations stay behind internal transaction ports.
use finance_core::prototype::{Error,recheck_authenticated,scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError,Role,lock_roles};
use finance_shared::{contracts::Principal,ids::{CompanyScope,ResourceScope}};
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone,Debug,Eq,PartialEq,sqlx::FromRow)]
pub struct ReservationRow{
    pub id:Uuid,pub snapshot_id:Uuid,pub resource_id:Uuid,pub amount:String,pub remaining:String,
    pub currency:String,pub state:String,pub version:i64,
}
#[derive(Clone,Debug,Eq,PartialEq,Serialize)]
pub struct MoneyDto{pub amount:String,pub currency:String}
#[derive(Clone,Debug,Eq,PartialEq,Serialize)]
pub struct Reservation{
    pub id:Uuid,pub snapshot_id:Uuid,pub resource_id:Uuid,pub amount:MoneyDto,pub remaining:MoneyDto,
    pub state:String,pub version:i64,
}
fn binding(proof:&AuthenticatedCommand)->Result<(Uuid,Uuid,Uuid),Error>{
    let ResourceScope::Company(CompanyScope{tenant,company})=proof.context().scope else{return Err(Error::Forbidden)};
    let Principal::User(actor)=proof.context().actor else{return Err(Error::Forbidden)};
    Ok((tenant.into(),company.into(),actor.into()))
}
fn grant(error:GrantError)->Error{match error{GrantError::Forbidden=>Error::Forbidden,GrantError::Database(e)=>Error::Database(e)}}
fn db(error:sqlx::Error)->Error{
    if error.as_database_error().and_then(|e|e.code()).as_deref()==Some("42501"){Error::Forbidden}else{Error::Database(error)}
}
fn map(row:ReservationRow)->Reservation{
    let currency=row.currency;
    Reservation{id:row.id,snapshot_id:row.snapshot_id,resource_id:row.resource_id,
      amount:MoneyDto{amount:row.amount,currency:currency.clone()},
      remaining:MoneyDto{amount:row.remaining,currency},state:row.state,version:row.version}
}
pub async fn list_reservations(
    pool:&PgPool,proof:&AuthenticatedCommand,dossier:Uuid,after:Uuid,limit:i64
)->Result<Vec<Reservation>,Error>{
    if dossier.is_nil()||!(1..=100).contains(&limit){return Err(Error::InvalidInput)}
    let(t,c,a)=binding(proof)?;let mut tx=scoped_transaction(pool,proof.context()).await?;
    let roles=lock_roles(&mut tx,proof.context()).await.map_err(grant)?;
    if !roles.contains(&Role::Requester)&&!roles.contains(&Role::Accountant){return Err(Error::Forbidden)}
    recheck_authenticated(&mut tx,Some(proof)).await?;
    let rows=sqlx::query_as::<_,ReservationRow>("SELECT * FROM finance.runtime_reservations($1,$2,$3,$4,$5,$6)")
      .bind(t).bind(c).bind(dossier).bind(a).bind(after).bind(limit).fetch_all(&mut *tx).await.map_err(db)?;
    recheck_authenticated(&mut tx,Some(proof)).await?;tx.commit().await?;
    Ok(rows.into_iter().map(map).collect())
}
