//! E20 template draft/read/revise routes.
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
use finance_shared::{
    contracts::{Principal, Version},
    http_contract::{IdempotencyKey, VersionTarget, VersionToken},
    ids::ResourceId,
};
use finance_templates::repository::{self, Template};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    policy_id: Uuid,
    name: String,
    document_id: Uuid,
    format: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revise {
    document_id: Uuid,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewBody {
    fixture_id: Uuid,
}

#[derive(Serialize)]
struct Page {
    items: Vec<Template>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

fn key(headers: &HeaderMap) -> Option<IdempotencyKey> {
    headers
        .get("idempotency-key")
        .and_then(|header| header.to_str().ok())
        .and_then(|value| IdempotencyKey::parse(value).ok())
}

fn receipt(template: Template, status: StatusCode) -> Response {
    let token = VersionToken {
        target: VersionTarget::Template,
        resource_id: ResourceId::try_from(template.id).expect("stored template id"),
        version: Version::try_from(template.version).expect("stored template version"),
    };
    let mut response = (status, Json(template)).into_response();
    response.headers_mut().insert(
        "finance-version",
        HeaderValue::from_str(&token.to_string()).expect("canonical header"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
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
    let Some(key) = key(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let draft = repository::CreateDraft {
        policy: body.policy_id,
        template: random_id(),
        row: random_id(),
        name: &body.name,
        document: body.document_id,
        format: &body.format,
    };
    match repository::create(&state.runtime, &proof, &key, draft).await {
        Ok(template) => receipt(template, StatusCode::CREATED),
        Err(error_value) => error(dossiers::status(error_value), correlation),
    }
}

pub(super) async fn revise(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(template): Path<Uuid>,
    body: Result<Json<Revise>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let Some(token) = headers
        .get("if-finance-version")
        .and_then(|header| header.to_str().ok())
        .and_then(|value| VersionToken::parse(value).ok())
    else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    if token.target != VersionTarget::Template || Uuid::from(token.resource_id) != template {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let proof =
        match dossiers::authenticate_version(&state, &headers, correlation, Some(token.version))
            .await
        {
            Ok(proof) => proof,
            Err(status) => return error(status, correlation),
        };
    let Some(key) = key(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let draft = repository::ReviseDraft {
        template,
        row: random_id(),
        document: body.document_id,
        reason: &body.reason,
    };
    match repository::revise(&state.runtime, &proof, &key, draft).await {
        Ok(template) => receipt(template, StatusCode::OK),
        Err(error_value) => error(dossiers::status(error_value), correlation),
    }
}

pub(super) async fn get(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(template): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    match repository::get(&state.runtime, &proof, template).await {
        Ok(Some(template)) => receipt(template, StatusCode::OK),
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(error_value) => error(dossiers::status(error_value), correlation),
    }
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
        Sha256::digest([b"finance.template.cursor/v1".as_slice(), &state.cursor_key].concat())
            .into();
    let after = match query.cursor {
        None => Uuid::nil(),
        Some(value) => match decode_cursor(&cursor_key, state.scope, actor, &value, time) {
            Some(id) => id,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    match repository::list(&state.runtime, &proof, after, limit).await {
        Ok(items) => {
            let next_cursor = if items.len() == limit as usize {
                items
                    .last()
                    .map(|item| encode_cursor(&cursor_key, state.scope, actor, item.id, time + 300))
            } else {
                None
            };
            let mut response = Json(Page { items, next_cursor }).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(error_value) => error(dossiers::status(error_value), correlation),
    }
}

pub(super) async fn preview(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(template): Path<Uuid>,
    body: Result<Json<PreviewBody>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let Some(token) = headers
        .get("if-finance-version")
        .and_then(|header| header.to_str().ok())
        .and_then(|value| VersionToken::parse(value).ok())
    else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    if token.target != VersionTarget::Template || Uuid::from(token.resource_id) != template {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let proof =
        match dossiers::authenticate_version(&state, &headers, correlation, Some(token.version))
            .await
        {
            Ok(proof) => proof,
            Err(status) => return error(status, correlation),
        };
    let Some(key) = key(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match repository::preview(
        &state.runtime,
        &proof,
        &key,
        template,
        body.fixture_id,
        random_id(),
    )
    .await
    {
        Ok(job) => {
            let version = Version::try_from(job.dossier_version).expect("stored template version");
            let token = VersionToken {
                target: VersionTarget::Template,
                resource_id: ResourceId::try_from(template).expect("stored template id"),
                version,
            };
            let mut response = (StatusCode::ACCEPTED, Json(job)).into_response();
            response.headers_mut().insert(
                "finance-version",
                HeaderValue::from_str(&token.to_string()).expect("canonical header"),
            );
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(error_value) => error(dossiers::status(error_value), correlation),
    }
}
