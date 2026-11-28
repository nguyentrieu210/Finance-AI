//! Verified invitation claim and independent organization administrator decision.
use super::{OrganizationState,dossiers,error,random_id};
use axum::{Json,extract::{State,Path,Query,rejection::JsonRejection},http::{HeaderMap,StatusCode,header},response::{IntoResponse,Response}};
use finance_core::prototype::{Error,scoped_transaction,recheck_authenticated};
use finance_identity::{AuthenticatedCommand,IdentityError};
use finance_permissions::{Role,GrantError,require_role,lock_roles};
use finance_shared::{contracts::Principal,ids::ResourceScope};
use serde::Deserialize;
use serde_json::{Value,json};
use sqlx::{Postgres,Transaction};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision { expected_version:i64, approve:bool }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Issue { expected_version:i64, issue:bool }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page { after:Option<Uuid>,limit:Option<i32> }
fn grant(e:GrantError)->Error{match e{GrantError::Forbidden=>Error::Forbidden,GrantError::Database(e)=>Error::Database(e)}}
fn db(e:sqlx::Error)->Error{match e.as_database_error().and_then(|d|d.code()).as_deref(){Some("42501")=>Error::Forbidden,Some("23514"|"23505"|"23503")=>Error::Conflict,_=>Error::Database(e)}}
fn binding(p:&AuthenticatedCommand)->Result<(Uuid,Uuid,Uuid),Error>{
 let ctx=p.context();let(ResourceScope::Company(s),Principal::User(a))=(ctx.scope,ctx.actor)else{return Err(Error::Forbidden)};
 Ok((s.tenant.into(),s.company.into(),a.into()))
}
async fn admin_tx(state:&OrganizationState,p:&AuthenticatedCommand)->Result<Transaction<'static,Postgres>,Error>{
 let mut tx=scoped_transaction(&state.runtime,p.context()).await?;
 require_role(&mut tx,p.context(),Role::OrgAdmin).await.map_err(grant)?;
 recheck_authenticated(&mut tx,Some(p)).await?;
 Ok(tx)
}
pub(super) async fn claim(State(state):State<Arc<OrganizationState>>,headers:HeaderMap)->Response{
 let corr=random_id();
 let Some(token)=headers.get(header::AUTHORIZATION).and_then(|h|h.to_str().ok()).and_then(|h|h.strip_prefix("Bearer ")).filter(|v|!v.is_empty()&&v.len()<=16384)else{return error(StatusCode::UNAUTHORIZED,corr)};
 match state.verifier.claim_invitation(&state.lookup,token,state.scope).await{
  Ok(Some(id))=>Json(json!({"id":id,"status":"PENDING"})).into_response(),
  Ok(None)|Err(IdentityError::Forbidden)=>error(StatusCode::FORBIDDEN,corr),
  Err(IdentityError::InvalidToken)=>error(StatusCode::UNAUTHORIZED,corr),
  Err(_)=>error(StatusCode::SERVICE_UNAVAILABLE,corr)
 }
}
pub(super) async fn list(State(state):State<Arc<OrganizationState>>,Query(page):Query<Page>,headers:HeaderMap)->Response{
 let corr=random_id();let p=match dossiers::authenticate_version(&state,&headers,corr,None).await{Ok(v)=>v,Err(s)=>return error(s,corr)};
 let n=page.limit.unwrap_or(100);if !(1..=100).contains(&n){return error(StatusCode::BAD_REQUEST,corr)}
 let result:Result<Value,Error>=async{
  let(t,c,a)=binding(&p)?;let mut tx=admin_tx(&state,&p).await?;
  let rows:Vec<String>=sqlx::query_scalar("SELECT x::text FROM finance.staff_onboarding_list($1,$2,$3,$4,$5) x")
   .bind(t).bind(c).bind(a).bind(page.after.unwrap_or(Uuid::nil())).bind(n).fetch_all(&mut *tx).await.map_err(db)?;
  let items:Vec<Value>=rows.iter().map(|s|serde_json::from_str(s).map_err(|_|Error::Conflict)).collect::<Result<_,_>>()?;
  let next=if items.len()==n as usize{items.last().and_then(|x|x.get("id")).cloned()}else{None};
  recheck_authenticated(&mut tx,Some(&p)).await?;tx.commit().await?;Ok(json!({"items":items,"next_after":next}))
 }.await;
 match result{Ok(v)=>Json(v).into_response(),Err(e)=>error(dossiers::status(e),corr)}
}
async fn transition(state:&OrganizationState,p:&AuthenticatedCommand,id:Uuid,version:i64,value:bool,issue:bool)->Result<i64,Error>{
 if id.is_nil()||!(0..=9_007_199_254_740_990).contains(&version){return Err(Error::InvalidInput)}
 let(t,c,a)=binding(p)?;let mut tx=admin_tx(state,p).await?;
 let stmt=if issue{"SELECT finance.staff_invitation_issue($1,$2,$3,$4,$5,$6)"}else{"SELECT finance.staff_onboarding_decide($1,$2,$3,$4,$5,$6)"};
 let result=sqlx::query_scalar(stmt).bind(t).bind(c).bind(a).bind(id).bind(version).bind(value).fetch_one(&mut *tx).await.map_err(db)?;
 recheck_authenticated(&mut tx,Some(p)).await?;tx.commit().await?;Ok(result)
}
pub(super) async fn issue(State(state):State<Arc<OrganizationState>>,Path(id):Path<Uuid>,headers:HeaderMap,body:Result<Json<Issue>,JsonRejection>)->Response{
 let corr=random_id();let p=match dossiers::authenticate_version(&state,&headers,corr,None).await{Ok(v)=>v,Err(s)=>return error(s,corr)};
 let Ok(Json(i))=body else{return error(StatusCode::BAD_REQUEST,corr)};
 match transition(&state,&p,id,i.expected_version,i.issue,true).await{Ok(v)=>Json(json!({"id":id,"version":v})).into_response(),Err(e)=>error(dossiers::status(e),corr)}
}
pub(super) async fn decide(State(state):State<Arc<OrganizationState>>,Path(id):Path<Uuid>,headers:HeaderMap,body:Result<Json<Decision>,JsonRejection>)->Response{
 let corr=random_id();let p=match dossiers::authenticate_version(&state,&headers,corr,None).await{Ok(v)=>v,Err(s)=>return error(s,corr)};
 let Ok(Json(i))=body else{return error(StatusCode::BAD_REQUEST,corr)};
 match transition(&state,&p,id,i.expected_version,i.approve,false).await{Ok(v)=>Json(json!({"id":id,"version":v})).into_response(),Err(e)=>error(dossiers::status(e),corr)}
}
/// Read-only effective capability preview. Not authorization for dossier mutations.
pub(super) async fn grants(State(state):State<Arc<OrganizationState>>,headers:HeaderMap)->Response{
 let corr=random_id();let p=match dossiers::authenticate_version(&state,&headers,corr,None).await{Ok(v)=>v,Err(s)=>return error(s,corr)};
 let result:Result<Value,Error>=async{
  let(t,c,a)=binding(&p)?;let mut tx=scoped_transaction(&state.runtime,p.context()).await?;
  if lock_roles(&mut tx,p.context()).await.map_err(grant)?.is_empty(){return Err(Error::Forbidden)}
  recheck_authenticated(&mut tx,Some(&p)).await?;
  let rows:Vec<(String,Option<Uuid>,i64,Option<i64>)>=sqlx::query_as("SELECT capability,department_id,valid_from,valid_until FROM finance.staff_effective_grants($1,$2,$3)").bind(t).bind(c).bind(a).fetch_all(&mut *tx).await.map_err(db)?;
  recheck_authenticated(&mut tx,Some(&p)).await?;tx.commit().await?;
  Ok(json!({"items":rows.into_iter().map(|(capability,department_id,valid_from,valid_until)|json!({"capability":capability,"department_id":department_id,"valid_from":valid_from,"valid_until":valid_until})).collect::<Vec<_>>(),"enforced":false}))
 }.await;
 match result{Ok(v)=>Json(v).into_response(),Err(e)=>error(dossiers::status(e),corr)}
}
