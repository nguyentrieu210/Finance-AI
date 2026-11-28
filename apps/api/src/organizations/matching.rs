//! E10 manual evidence-pair confirmation.
use super::{OrganizationState,PageQuery,decode_cursor,dossiers,encode_cursor,error,now,random_id};
use axum::{Json,extract::{Path,Query,State,rejection::{JsonRejection,QueryRejection}},http::{HeaderMap,HeaderValue,StatusCode,header},response::{IntoResponse,Response}};
use finance_matching::repository::{self,Match};
use finance_shared::{contracts::{Principal,Version},http_contract::{IdempotencyKey,VersionTarget,VersionToken},ids::ResourceId};
use serde::Deserialize;use sha2::{Digest,Sha256};use std::sync::Arc;use uuid::Uuid;
#[derive(serde::Serialize)]struct Page{items:Vec<Match>,#[serde(skip_serializing_if="Option::is_none")]next_cursor:Option<String>}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]struct Body{left_evidence_id:Uuid,right_evidence_id:Uuid,reason:String}
fn key(h:&HeaderMap)->Option<IdempotencyKey>{h.get("idempotency-key").and_then(|v|v.to_str().ok()).and_then(|v|IdempotencyKey::parse(v).ok())}
pub(super)async fn list(State(state):State<Arc<OrganizationState>>,headers:HeaderMap,Path(dossier):Path<Uuid>,query:Result<Query<PageQuery>,QueryRejection>)->Response{
 let correlation=random_id();let proof=match dossiers::authenticate_version(&state,&headers,correlation,None).await{Ok(v)=>v,Err(s)=>return error(s,correlation)};
 let Principal::User(actor)=proof.context().actor else{return error(StatusCode::FORBIDDEN,correlation)};let Ok(Query(q))=query else{return error(StatusCode::BAD_REQUEST,correlation)};let limit=q.limit.unwrap_or(50);if !(1..=100).contains(&limit){return error(StatusCode::BAD_REQUEST,correlation)}
 let Some(time)=now()else{return error(StatusCode::SERVICE_UNAVAILABLE,correlation)};let ck:[u8;32]=Sha256::digest([b"finance.match.cursor/v1".as_slice(),&state.cursor_key,dossier.as_bytes()].concat()).into();
 let after=match q.cursor{None=>Uuid::nil(),Some(v)=>match decode_cursor(&ck,state.scope,actor,&v,time){Some(id)=>id,None=>return error(StatusCode::BAD_REQUEST,correlation)}};
 match repository::list(&state.runtime,&proof,dossier,after,limit).await{Ok(items)=>{let next_cursor=if items.len()==limit as usize{items.last().map(|x|encode_cursor(&ck,state.scope,actor,x.id,time+300))}else{None};Json(Page{items,next_cursor}).into_response()},Err(e)=>error(dossiers::status(e),correlation)}
}
pub(super)async fn confirm(State(state):State<Arc<OrganizationState>>,headers:HeaderMap,Path(dossier):Path<Uuid>,body:Result<Json<Body>,JsonRejection>)->Response{
 let correlation=random_id();let Some(token)=headers.get("if-finance-version").and_then(|v|v.to_str().ok()).and_then(|v|VersionToken::parse(v).ok())else{return error(StatusCode::BAD_REQUEST,correlation)};
 if token.target!=VersionTarget::Dossier||Uuid::from(token.resource_id)!=dossier{return error(StatusCode::BAD_REQUEST,correlation)}
 let proof=match dossiers::authenticate_version(&state,&headers,correlation,Some(token.version)).await{Ok(v)=>v,Err(s)=>return error(s,correlation)};
 let Some(key)=key(&headers)else{return error(StatusCode::BAD_REQUEST,correlation)};let Ok(Json(b))=body else{return error(StatusCode::BAD_REQUEST,correlation)};
 match repository::confirm(&state.runtime,&proof,&key,dossier,b.left_evidence_id,b.right_evidence_id,&b.reason,random_id(),random_id()).await{
  Ok(receipt)=>{let Ok(version)=Version::try_from(receipt.dossier_version)else{return error(StatusCode::SERVICE_UNAVAILABLE,correlation)};let token=VersionToken{target:VersionTarget::Dossier,resource_id:ResourceId::try_from(dossier).expect("path dossier"),version};let mut out=Json(receipt.item).into_response();out.headers_mut().insert("finance-version",HeaderValue::from_str(&token.to_string()).expect("canonical"));out.headers_mut().insert(header::CACHE_CONTROL,HeaderValue::from_static("no-store"));out}
  Err(e)=>error(dossiers::status(e),correlation)
 }
}
