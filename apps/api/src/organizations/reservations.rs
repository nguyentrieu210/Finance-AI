//! E11 read-only reservation projection.
use super::{OrganizationState,PageQuery,decode_cursor,dossiers,encode_cursor,error,now,random_id};
use axum::{Json,extract::{Path,Query,State,rejection::QueryRejection},http::{HeaderMap,StatusCode},response::{IntoResponse,Response}};
use finance_shared::contracts::Principal;use serde::Serialize;use sha2::{Digest,Sha256};use std::sync::Arc;use uuid::Uuid;
#[derive(Serialize)]struct Page{items:Vec<finance_financial::repository::Reservation>,#[serde(skip_serializing_if="Option::is_none")]next_cursor:Option<String>}
pub(super)async fn list(State(state):State<Arc<OrganizationState>>,headers:HeaderMap,Path(dossier):Path<Uuid>,query:Result<Query<PageQuery>,QueryRejection>)->Response{
 let correlation=random_id();let proof=match dossiers::authenticate_version(&state,&headers,correlation,None).await{Ok(v)=>v,Err(s)=>return error(s,correlation)};
 let Principal::User(actor)=proof.context().actor else{return error(StatusCode::FORBIDDEN,correlation)};let Ok(Query(q))=query else{return error(StatusCode::BAD_REQUEST,correlation)};let limit=q.limit.unwrap_or(50);if !(1..=100).contains(&limit){return error(StatusCode::BAD_REQUEST,correlation)}
 let Some(time)=now()else{return error(StatusCode::SERVICE_UNAVAILABLE,correlation)};let ck:[u8;32]=Sha256::digest([b"finance.reservation.cursor/v1".as_slice(),&state.cursor_key,dossier.as_bytes()].concat()).into();
 let after=match q.cursor{None=>Uuid::nil(),Some(v)=>match decode_cursor(&ck,state.scope,actor,&v,time){Some(id)=>id,None=>return error(StatusCode::BAD_REQUEST,correlation)}};
 match finance_financial::repository::list_reservations(&state.runtime,&proof,dossier,after,limit).await{Ok(items)=>{let next_cursor=if items.len()==limit as usize{items.last().map(|x|encode_cursor(&ck,state.scope,actor,x.id,time+300))}else{None};Json(Page{items,next_cursor}).into_response()},Err(e)=>error(dossiers::status(e),correlation)}
}
