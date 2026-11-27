//! E08/E09 current extraction reads and independent field decisions.
use super::{OrganizationState,PageQuery,decode_cursor,dossiers,encode_cursor,error,now,random_id};
use axum::{
    Json,
    extract::{Path,Query,State,rejection::{JsonRejection,QueryRejection}},
    http::{HeaderMap,HeaderValue,StatusCode,header},
    response::{IntoResponse,Response},
};
use finance_extraction::repository::{self,Field};
use finance_shared::{
    contracts::{Principal,Version},
    http_contract::{IdempotencyKey,VersionTarget,VersionToken},
    ids::ResourceId,
};
use serde::Deserialize;
use sha2::{Digest,Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[derive(serde::Serialize)]
struct Page {
    items: Vec<Field>,
    #[serde(skip_serializing_if="Option::is_none")]
    next_cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Decision {
    field_id: Uuid,
    value: String,
    reason: String,
    evidence_ids: Vec<Uuid>,
}
fn key(headers:&HeaderMap)->Option<IdempotencyKey>{
    headers.get("idempotency-key").and_then(|v|v.to_str().ok()).and_then(|v|IdempotencyKey::parse(v).ok())
}
fn version_proof(
    token:VersionToken,
    dossier:Uuid,
)->Result<Version,StatusCode>{
    if token.target!=VersionTarget::Dossier||Uuid::from(token.resource_id)!=dossier{
        Err(StatusCode::BAD_REQUEST)
    }else{
        Ok(token.version)
    }
}
pub(super) async fn list(
    State(state):State<Arc<OrganizationState>>,
    headers:HeaderMap,
    Path(dossier):Path<Uuid>,
    query:Result<Query<PageQuery>,QueryRejection>,
)->Response{
    let correlation=random_id();
    let proof=match dossiers::authenticate_version(&state,&headers,correlation,None).await{
        Ok(p)=>p,Err(s)=>return error(s,correlation)
    };
    let Principal::User(actor)=proof.context().actor else{return error(StatusCode::FORBIDDEN,correlation)};
    let Ok(Query(query))=query else{return error(StatusCode::BAD_REQUEST,correlation)};
    let limit=query.limit.unwrap_or(50);
    if dossier.is_nil()||!(1..=100).contains(&limit){return error(StatusCode::BAD_REQUEST,correlation)}
    let Some(time)=now()else{return error(StatusCode::SERVICE_UNAVAILABLE,correlation)};
    let cursor_key:[u8;32]=Sha256::digest([b"finance.field.cursor/v1".as_slice(),&state.cursor_key,dossier.as_bytes()].concat()).into();
    let after=match query.cursor{
        None=>Uuid::nil(),
        Some(value)=>match decode_cursor(&cursor_key,state.scope,actor,&value,time){
            Some(id)=>id,None=>return error(StatusCode::BAD_REQUEST,correlation)
        }
    };
    match repository::list(&state.runtime,&proof,dossier,after,limit).await{
        Ok(items)=>{
            let next_cursor=if items.len()==limit as usize{
                items.last().map(|field|encode_cursor(&cursor_key,state.scope,actor,field.id,time+300))
            }else{None};
            let mut response=Json(Page{items,next_cursor}).into_response();
            response.headers_mut().insert(header::CACHE_CONTROL,HeaderValue::from_static("no-store"));
            response
        }
        Err(value)=>error(dossiers::status(value),correlation),
    }
}
pub(super) async fn decide(
    State(state):State<Arc<OrganizationState>>,
    headers:HeaderMap,
    Path(dossier):Path<Uuid>,
    body:Result<Json<Decision>,JsonRejection>,
)->Response{
    let correlation=random_id();
    let Some(token)=headers.get("if-finance-version").and_then(|v|v.to_str().ok()).and_then(|v|VersionToken::parse(v).ok())
    else{return error(StatusCode::BAD_REQUEST,correlation)};
    let version=match version_proof(token,dossier){Ok(v)=>v,Err(s)=>return error(s,correlation)};
    let proof=match dossiers::authenticate_version(&state,&headers,correlation,Some(version)).await{
        Ok(p)=>p,Err(s)=>return error(s,correlation)
    };
    let Some(key)=key(&headers)else{return error(StatusCode::BAD_REQUEST,correlation)};
    let Ok(Json(body))=body else{return error(StatusCode::BAD_REQUEST,correlation)};
    match finance_validation::repository::decide(
        &state.runtime,&proof,&key,dossier,body.field_id,&body.value,&body.reason,&body.evidence_ids,random_id(),
    ).await{
        Ok(receipt)=>{
            let Ok(version)=Version::try_from(receipt.dossier_version)else{return error(StatusCode::SERVICE_UNAVAILABLE,correlation)};
            let token=VersionToken{target:VersionTarget::Dossier,resource_id:ResourceId::try_from(dossier).expect("path dossier"),version};
            let mut response=(StatusCode::OK,Json(receipt.field)).into_response();
            response.headers_mut().insert("finance-version",HeaderValue::from_str(&token.to_string()).expect("canonical"));
            response.headers_mut().insert(header::CACHE_CONTROL,HeaderValue::from_static("no-store"));
            response
        }
        Err(value)=>error(dossiers::status(value),correlation),
    }
}
