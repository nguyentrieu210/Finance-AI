//! E19 form draft/read/revise routes.
use super::{OrganizationState, PageQuery, decode_cursor, dossiers, encode_cursor, error, now, random_id};
use axum::{
    Json,
    extract::{Path, Query, State, rejection::{JsonRejection, QueryRejection}},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use finance_forms::repository::{self, DraftField, Form};
use finance_shared::{
    contracts::{Principal, Version},
    http_contract::{IdempotencyKey, VersionTarget, VersionToken},
    ids::ResourceId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    policy_id: Uuid,
    name: String,
    fields: Vec<DraftField>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revise {
    name: String,
    fields: Vec<DraftField>,
    reason: String,
}
#[derive(Serialize)]
struct Page {
    items: Vec<Form>,
    #[serde(skip_serializing_if="Option::is_none")]
    next_cursor: Option<String>,
}
fn receipt(form: Form, status: StatusCode) -> Response {
    let token=VersionToken{
        target:VersionTarget::Form,
        resource_id:ResourceId::try_from(form.id).expect("stored form id"),
        version:Version::try_from(form.version).expect("stored form version"),
    };
    let mut response=(status,Json(form)).into_response();
    response.headers_mut().insert("finance-version",HeaderValue::from_str(&token.to_string()).expect("canonical header"));
    response.headers_mut().insert(header::CACHE_CONTROL,HeaderValue::from_static("no-store"));
    response
}
fn key(headers:&HeaderMap)->Option<IdempotencyKey>{
    headers.get("idempotency-key").and_then(|h|h.to_str().ok()).and_then(|s|IdempotencyKey::parse(s).ok())
}
pub(super) async fn create(
    State(state):State<Arc<OrganizationState>>,headers:HeaderMap,
    body:Result<Json<Create>,JsonRejection>
)->Response{
    let correlation=random_id();
    let proof=match dossiers::authenticate_version(&state,&headers,correlation,None).await{Ok(p)=>p,Err(s)=>return error(s,correlation)};
    let Some(key)=key(&headers) else{return error(StatusCode::BAD_REQUEST,correlation)};
    let Ok(Json(body))=body else{return error(StatusCode::BAD_REQUEST,correlation)};
    match repository::create(&state.runtime,&proof,&key,body.policy_id,random_id(),random_id(),&body.name,&body.fields).await{
        Ok(form)=>receipt(form,StatusCode::CREATED),
        Err(e)=>error(dossiers::status(e),correlation),
    }
}
pub(super) async fn revise(
    State(state):State<Arc<OrganizationState>>,headers:HeaderMap,Path(form):Path<Uuid>,
    body:Result<Json<Revise>,JsonRejection>
)->Response{
    let correlation=random_id();
    let Some(token)=headers.get("if-finance-version").and_then(|h|h.to_str().ok()).and_then(|s|VersionToken::parse(s).ok()) else{
        return error(StatusCode::BAD_REQUEST,correlation);
    };
    if token.target!=VersionTarget::Form || Uuid::from(token.resource_id)!=form{
        return error(StatusCode::BAD_REQUEST,correlation);
    }
    let proof=match dossiers::authenticate_version(&state,&headers,correlation,Some(token.version)).await{Ok(p)=>p,Err(s)=>return error(s,correlation)};
    let Some(key)=key(&headers) else{return error(StatusCode::BAD_REQUEST,correlation)};
    let Ok(Json(body))=body else{return error(StatusCode::BAD_REQUEST,correlation)};
    match repository::revise(&state.runtime,&proof,&key,form,random_id(),&body.name,&body.fields,&body.reason).await{
        Ok(form)=>receipt(form,StatusCode::OK),
        Err(e)=>error(dossiers::status(e),correlation),
    }
}
pub(super) async fn get(
    State(state):State<Arc<OrganizationState>>,headers:HeaderMap,Path(form):Path<Uuid>
)->Response{
    let correlation=random_id();
    let proof=match dossiers::authenticate_version(&state,&headers,correlation,None).await{Ok(p)=>p,Err(s)=>return error(s,correlation)};
    match repository::get(&state.runtime,&proof,form).await{
        Ok(Some(form))=>receipt(form,StatusCode::OK),
        Ok(None)=>error(StatusCode::NOT_FOUND,correlation),
        Err(e)=>error(dossiers::status(e),correlation),
    }
}
pub(super) async fn list(
    State(state):State<Arc<OrganizationState>>,headers:HeaderMap,
    query:Result<Query<PageQuery>,QueryRejection>
)->Response{
    let correlation=random_id();
    let proof=match dossiers::authenticate_version(&state,&headers,correlation,None).await{Ok(p)=>p,Err(s)=>return error(s,correlation)};
    let Principal::User(actor)=proof.context().actor else{return error(StatusCode::FORBIDDEN,correlation)};
    let Ok(Query(query))=query else{return error(StatusCode::BAD_REQUEST,correlation)};
    let limit=query.limit.unwrap_or(50);if !(1..=100).contains(&limit){return error(StatusCode::BAD_REQUEST,correlation);}
    let Some(time)=now() else{return error(StatusCode::SERVICE_UNAVAILABLE,correlation)};
    let cursor_key:[u8;32]=Sha256::digest([b"finance.form.cursor/v1".as_slice(),&state.cursor_key].concat()).into();
    let after=match query.cursor{None=>Uuid::nil(),Some(value)=>match decode_cursor(&cursor_key,state.scope,actor,&value,time){Some(id)=>id,None=>return error(StatusCode::BAD_REQUEST,correlation)}};
    match repository::list(&state.runtime,&proof,after,limit).await{
        Ok(items)=>{
            let next_cursor=if items.len()==limit as usize{items.last().map(|item|encode_cursor(&cursor_key,state.scope,actor,item.id,time+300))}else{None};
            let mut response=Json(Page{items,next_cursor}).into_response();
            response.headers_mut().insert(header::CACHE_CONTROL,HeaderValue::from_static("no-store"));response
        }
        Err(e)=>error(dossiers::status(e),correlation),
    }
}
