//! Published policy read API. Draft governance mutations remain closed.
use super::{
    OrganizationState, PageQuery, decode_cursor, dossiers, encode_cursor, error, now, random_id,
};
use axum::{
    Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use finance_policy::repository::{self, PublishedPolicy};
use finance_shared::{
    contracts::{Principal, Version},
    http_contract::{IdempotencyKey, VersionTarget, VersionToken},
    ids::ResourceId,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Serialize)]
struct Page {
    items: Vec<PublishedPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}
fn receipt(policy: PublishedPolicy) -> Response {
    let token = VersionToken {
        target: VersionTarget::Policy,
        resource_id: ResourceId::try_from(policy.id).expect("stored nonnil policy"),
        version: Version::try_from(policy.version).expect("stored policy version"),
    };
    let mut response = Json(policy).into_response();
    response.headers_mut().insert(
        "finance-version",
        HeaderValue::from_str(&token.to_string()).expect("canonical header"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
pub(super) async fn list(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
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
    let key: [u8; 32] =
        Sha256::digest([b"finance.policy.cursor/v1".as_slice(), &state.cursor_key].concat()).into();
    let after = match query.cursor {
        None => Uuid::nil(),
        Some(value) => match decode_cursor(&key, state.scope, actor, &value, time) {
            Some(id) => id,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    match repository::list(&state.runtime, &proof, after, limit).await {
        Ok(items) => {
            let next_cursor = if items.len() == limit as usize {
                items
                    .last()
                    .map(|p| encode_cursor(&key, state.scope, actor, p.id, time + 300))
            } else {
                None
            };
            let mut response = Json(Page { items, next_cursor }).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(e) => error(dossiers::status(e), correlation),
    }
}
pub(super) async fn get(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(p) => p,
        Err(s) => return error(s, correlation),
    };
    match repository::get(&state.runtime, &proof, id).await {
        Ok(Some(policy)) => receipt(policy),
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(e) => error(dossiers::status(e), correlation),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reason {
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Publish {
    reason: String,
    effective_from: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Revise {
    name: String,
    reason: String,
    effective_from: chrono::DateTime<chrono::Utc>,
}

async fn change(
    state: Arc<OrganizationState>,
    headers: HeaderMap,
    policy: Uuid,
    operation: repository::Change,
    reason: String,
    name: Option<String>,
    effective: Option<chrono::DateTime<chrono::Utc>>,
) -> Response {
    let correlation = random_id();
    let Some(token) = headers
        .get("if-finance-version")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| VersionToken::parse(s).ok())
    else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    if token.target != VersionTarget::Policy || Uuid::from(token.resource_id) != policy {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let proof =
        match dossiers::authenticate_version(&state, &headers, correlation, Some(token.version))
            .await
        {
            Ok(proof) => proof,
            Err(s) => return error(s, correlation),
        };
    let Some(key) = headers
        .get("idempotency-key")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| IdempotencyKey::parse(s).ok())
    else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let input = repository::ChangeInput {
        policy,
        new_id: random_id(),
        change: operation,
        reason: &reason,
        name: name.as_deref(),
        effective,
    };
    match repository::change(&state.runtime, &proof, &key, input).await {
        Ok(row) => receipt(row),
        Err(e) => error(dossiers::status(e), correlation),
    }
}

pub(super) async fn submit(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(policy): Path<Uuid>,
    body: Result<Json<Reason>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, random_id());
    };
    change(
        state,
        headers,
        policy,
        repository::Change::Submit,
        body.reason,
        None,
        None,
    )
    .await
}

pub(super) async fn retire(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(policy): Path<Uuid>,
    body: Result<Json<Reason>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, random_id());
    };
    change(
        state,
        headers,
        policy,
        repository::Change::Retire,
        body.reason,
        None,
        None,
    )
    .await
}

pub(super) async fn revoke(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(policy): Path<Uuid>,
    body: Result<Json<Reason>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, random_id());
    };
    change(
        state,
        headers,
        policy,
        repository::Change::Revoke,
        body.reason,
        None,
        None,
    )
    .await
}

pub(super) async fn publish(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(policy): Path<Uuid>,
    body: Result<Json<Publish>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, random_id());
    };
    change(
        state,
        headers,
        policy,
        repository::Change::Publish,
        body.reason,
        None,
        Some(body.effective_from),
    )
    .await
}

pub(super) async fn revise(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(policy): Path<Uuid>,
    body: Result<Json<Revise>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, random_id());
    };
    change(
        state,
        headers,
        policy,
        repository::Change::Revise,
        body.reason,
        Some(body.name),
        Some(body.effective_from),
    )
    .await
}
