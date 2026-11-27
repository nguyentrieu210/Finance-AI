//! E18 accountant review HTTP routes.
use super::{
    OrganizationState, PageQuery, decode_cursor, dossiers, encode_cursor, error, now, random_id,
};
use axum::{
    Json,
    extract::{Path, Query, State, rejection::{JsonRejection, QueryRejection}},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use finance_shared::{
    contracts::{Principal, Version},
    http_contract::{IdempotencyKey, VersionTarget, VersionToken},
    ids::ResourceId,
};
use finance_workflow::review;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Begin {
    reason: String,
    evidence_ids: Vec<Uuid>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    snapshot_id: Uuid,
    decision: String,
    confirmed_field_ids: Vec<Uuid>,
    reason: String,
}

#[derive(Serialize)]
struct Queue {
    items: Vec<finance_workflow::draft::Draft>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

fn token(headers: &HeaderMap, dossier: Uuid) -> Option<Version> {
    let token = headers
        .get("if-finance-version")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| VersionToken::parse(value).ok())?;
    if token.target != VersionTarget::Dossier || Uuid::from(token.resource_id) != dossier {
        return None;
    }
    Some(token.version)
}

fn key(headers: &HeaderMap) -> Option<IdempotencyKey> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| IdempotencyKey::parse(value).ok())
}

fn versioned<T: Serialize>(dossier: Uuid, version: i64, status: StatusCode, body: T) -> Response {
    let Ok(version) = Version::try_from(version) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, random_id());
    };
    let token = VersionToken {
        target: VersionTarget::Dossier,
        resource_id: ResourceId::try_from(dossier).expect("stored nonnil dossier"),
        version,
    };
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        "finance-version",
        HeaderValue::from_str(&token.to_string()).expect("canonical version"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(super) async fn begin(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
    body: Result<Json<Begin>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let Some(expected) = token(&headers, dossier) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, Some(expected)).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    let Some(key) = key(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match review::begin(
        &state.runtime,
        &proof,
        &key,
        dossier,
        random_id(),
        &body.reason,
        &body.evidence_ids,
    )
    .await
    {
        Ok(receipt) => versioned(dossier, receipt.version, StatusCode::OK, receipt),
        Err(value) => error(dossiers::status(value), correlation),
    }
}

pub(super) async fn get(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    match review::get(&state.runtime, &proof, dossier).await {
        Ok(Some(value)) => {
            let mut response = Json(value).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(value) => error(dossiers::status(value), correlation),
    }
}

pub(super) async fn decide(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
    body: Result<Json<Decision>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let Some(expected) = token(&headers, dossier) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, Some(expected)).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    let Some(key) = key(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match review::decide(
        &state.runtime,
        &proof,
        &key,
        dossier,
        body.snapshot_id,
        &body.decision,
        &body.confirmed_field_ids,
        &body.reason,
    )
    .await
    {
        Ok(value) => versioned(dossier, value.dossier_version, StatusCode::OK, value),
        Err(value) => error(dossiers::status(value), correlation),
    }
}

pub(super) async fn queue(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
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
        [b"finance.review.queue/v1".as_slice(), &state.cursor_key].concat(),
    )
    .into();
    let after = match query.cursor {
        None => Uuid::nil(),
        Some(value) => match decode_cursor(&cursor_key, state.scope, actor, &value, time) {
            Some(id) => id,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    match review::queue(&state.runtime, &proof, after, limit).await {
        Ok(items) => {
            let next_cursor = if items.len() == limit as usize {
                items.last().map(|item| {
                    encode_cursor(&cursor_key, state.scope, actor, item.id, time + 300)
                })
            } else {
                None
            };
            let mut response = Json(Queue { items, next_cursor }).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(value) => error(dossiers::status(value), correlation),
    }
}
