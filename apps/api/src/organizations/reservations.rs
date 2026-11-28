//! E11 read-only reservation HTTP projection.
use super::{
    OrganizationState, PageQuery, decode_cursor, dossiers, encode_cursor, error, now, random_id,
};
use axum::{
    Json,
    extract::{Path, Query, State, rejection::QueryRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use finance_shared::contracts::Principal;
use serde::{Deserialize,Serialize};
use finance_shared::{contracts::Version,http_contract::{IdempotencyKey,VersionToken,VersionTarget},ids::ResourceId};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Serialize)]
struct Money {
    amount: String,
    currency: String,
}
#[derive(Serialize)]
struct Reservation {
    id: Uuid,
    snapshot_id: Option<Uuid>,
    resource_id: Uuid,
    resource_ids: Vec<Uuid>,
    revision_id: Uuid,
    policy_id: Option<Uuid>,
    amount: Money,
    remaining: Money,
    state: String,
    version: i64,
}
#[derive(Serialize)]
struct Page {
    items: Vec<Reservation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}
pub(super) async fn list(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(p) => p,
        Err(s) => return error(s, correlation),
    };
    let Principal::User(actor) = proof.context().actor else {
        return error(StatusCode::FORBIDDEN, correlation);
    };
    let Ok(Query(query)) = query else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let Some(time) = now() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let cursor_key: [u8; 32] = Sha256::digest(
        [
            b"finance.reservation.cursor/v1".as_slice(),
            &state.cursor_key,
            dossier.as_bytes(),
        ]
        .concat(),
    )
    .into();
    let after = match query.cursor {
        None => Uuid::nil(),
        Some(value) => match decode_cursor(&cursor_key, state.scope, actor, &value, time) {
            Some(id) => id,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    match finance_financial::reservations::list(&state.runtime, &proof, dossier, after, limit).await
    {
        Ok(rows) => {
            let next_cursor = if rows.len() == limit as usize {
                rows.last()
                    .map(|row| encode_cursor(&cursor_key, state.scope, actor, row.id, time + 300))
            } else {
                None
            };
            let items = rows
                .into_iter()
                .map(|row| Reservation {
                    id: row.id,
                    snapshot_id: row.snapshot_id,
                    resource_id: row.resource_id,
                    resource_ids: row.resource_ids,
                    revision_id: row.revision_id,
                    policy_id: row.policy_id,
                    amount: Money {
                        amount: row.amount,
                        currency: row.currency.clone(),
                    },
                    remaining: Money {
                        amount: row.remaining,
                        currency: row.currency,
                    },
                    state: row.state,
                    version: row.version,
                })
                .collect();
            let mut response = Json(Page { items, next_cursor }).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(value) => error(dossiers::status(value), correlation),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Bind {policy_id:Uuid,resource_ids:Vec<Uuid>,reason:String}
pub(super) async fn bind(State(state):State<Arc<OrganizationState>>,headers:HeaderMap,Path(dossier):Path<Uuid>,body:Result<Json<Bind>,axum::extract::rejection::JsonRejection>)->Response {
 let correlation=random_id();
 let Some(token)=headers.get("if-finance-version").and_then(|h|h.to_str().ok()).and_then(|s|VersionToken::parse(s).ok()) else{return error(StatusCode::BAD_REQUEST,correlation);};
 if token.target!=VersionTarget::Dossier || Uuid::from(token.resource_id)!=dossier{return error(StatusCode::BAD_REQUEST,correlation);}
 let proof=match dossiers::authenticate_version(&state,&headers,correlation,Some(token.version)).await{Ok(p)=>p,Err(s)=>return error(s,correlation)};
 let Some(key)=headers.get("idempotency-key").and_then(|h|h.to_str().ok()).and_then(|s|IdempotencyKey::parse(s).ok())else{return error(StatusCode::BAD_REQUEST,correlation);};
 let Ok(Json(body))=body else{return error(StatusCode::BAD_REQUEST,correlation);};
 match finance_financial::reservations::bind(&state.runtime,&proof,&key,dossier,body.policy_id,&body.resource_ids,&body.reason,random_id()).await{
 Ok(row)=>{
 let version=VersionToken{target:VersionTarget::Dossier,resource_id:ResourceId::try_from(dossier).expect("validated dossier"),version:Version::try_from(row.dossier_version).expect("stored version")};
 let mut response=(StatusCode::CREATED,Json(row)).into_response();
 response.headers_mut().insert("finance-version",HeaderValue::from_str(&version.to_string()).expect("canonical header"));
 response.headers_mut().insert(header::CACHE_CONTROL,HeaderValue::from_static("no-store"));
 response
 },Err(e)=>error(dossiers::status(e),correlation)}
}
