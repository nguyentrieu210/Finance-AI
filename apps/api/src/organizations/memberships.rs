//! E02/E03 membership administration HTTP routes.
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
use finance_organization::memberships::{self, Membership};
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
struct Item {
    id: Uuid,
    subject_id: Uuid,
    company_id: Uuid,
    roles: Vec<String>,
    active: bool,
}

impl From<Membership> for Item {
    fn from(value: Membership) -> Self {
        Self {
            id: value.id,
            subject_id: value.subject_id,
            company_id: value.company_id,
            roles: value.roles,
            active: value.active,
        }
    }
}

#[derive(Serialize)]
struct Page {
    items: Vec<Item>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    subject_id: Uuid,
    company_id: Uuid,
    roles: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revoke {
    reason: String,
}

fn idempotency(headers: &HeaderMap) -> Option<IdempotencyKey> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| IdempotencyKey::parse(value).ok())
}

fn expected(headers: &HeaderMap, id: Uuid) -> Option<Version> {
    let token = headers
        .get("if-finance-version")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| VersionToken::parse(value).ok())?;
    if token.target != VersionTarget::Membership || Uuid::from(token.resource_id) != id {
        return None;
    }
    Some(token.version)
}

fn receipt(value: Membership, status: StatusCode) -> Response {
    let token = VersionToken {
        target: VersionTarget::Membership,
        resource_id: ResourceId::try_from(value.id).expect("stored nonnil membership"),
        version: Version::try_from(value.version).expect("stored membership version"),
    };
    let mut response = (status, Json(Item::from(value))).into_response();
    response.headers_mut().insert(
        "finance-version",
        HeaderValue::from_str(&token.to_string()).expect("canonical membership version"),
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
    let cursor_key: [u8; 32] =
        Sha256::digest([b"finance.membership.cursor/v1".as_slice(), &state.cursor_key].concat())
            .into();
    let after = match query.cursor {
        None => Uuid::nil(),
        Some(value) => match decode_cursor(&cursor_key, state.scope, actor, &value, time) {
            Some(id) => id,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    match memberships::list(&state.runtime, &proof, after, limit).await {
        Ok(rows) => {
            let next_cursor = if rows.len() == limit as usize {
                rows.last()
                    .map(|row| encode_cursor(&cursor_key, state.scope, actor, row.id, time + 300))
            } else {
                None
            };
            let items = rows.into_iter().map(Item::from).collect();
            let mut response = Json(Page { items, next_cursor }).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(value) => error(dossiers::status(value), correlation),
    }
}

pub(super) async fn get(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    match memberships::get(&state.runtime, &proof, id).await {
        Ok(Some(value)) => receipt(value, StatusCode::OK),
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(value) => error(dossiers::status(value), correlation),
    }
}

pub(super) async fn create(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    body: Result<Json<Create>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    let Some(key) = idempotency(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    if body.company_id != Uuid::from(state.scope.company) {
        return error(StatusCode::FORBIDDEN, correlation);
    }
    match memberships::create(
        &state.runtime,
        &proof,
        &key,
        body.subject_id,
        body.company_id,
        &body.roles,
    )
    .await
    {
        Ok(value) => receipt(value, StatusCode::CREATED),
        Err(value) => error(dossiers::status(value), correlation),
    }
}

pub(super) async fn revoke(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: Result<Json<Revoke>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let Some(version) = expected(&headers, id) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let proof =
        match dossiers::authenticate_version(&state, &headers, correlation, Some(version)).await {
            Ok(proof) => proof,
            Err(status) => return error(status, correlation),
        };
    let Some(key) = idempotency(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match memberships::revoke(&state.runtime, &proof, &key, id, &body.reason).await {
        Ok(value) => receipt(value, StatusCode::OK),
        Err(value) => error(dossiers::status(value), correlation),
    }
}
