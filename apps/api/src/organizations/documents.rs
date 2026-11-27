//! Metadata-only document routes in the injected synthetic router.
use super::dossiers::{authenticate_version, status};
use super::{OrganizationState, decode_cursor, encode_cursor, error, now, random_id};
use axum::{
    Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use finance_documents::repository::{self, Document, Metadata};
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
    file_name: String,
    mime: String,
    bytes: u64,
    sha256: String,
    kind: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct List {
    limit: Option<i64>,
    cursor: Option<String>,
}

#[derive(Serialize)]
struct Page {
    items: Vec<Document>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

pub async fn create(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
    body: Result<Json<Create>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let Some(token) = headers
        .get("if-finance-version")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| VersionToken::parse(s).ok())
    else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    if token.target != VersionTarget::Dossier || Uuid::from(token.resource_id) != dossier {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let proof = match authenticate_version(&state, &headers, correlation, Some(token.version)).await
    {
        Ok(p) => p,
        Err(s) => return error(s, correlation),
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Some(key) = headers
        .get("idempotency-key")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| IdempotencyKey::parse(s).ok())
    else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let input = Metadata {
        filename: &body.file_name,
        mime: &body.mime,
        bytes: body.bytes,
        sha256: &body.sha256,
        kind: &body.kind,
    };
    match repository::create(&state.runtime, &proof, &key, dossier, random_id(), &input).await {
        Ok(receipt) => {
            let token = VersionToken {
                target: VersionTarget::Dossier,
                resource_id: ResourceId::try_from(dossier).expect("stored nonnil ID"),
                version: Version::try_from(receipt.version).expect("stored version"),
            };
            let mut response = (StatusCode::CREATED, Json(receipt)).into_response();
            response.headers_mut().insert(
                "finance-version",
                HeaderValue::from_str(&token.to_string()).expect("canonical header"),
            );
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(e) => error(status(e), correlation),
    }
}

pub async fn list(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
    query: Result<Query<List>, QueryRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match authenticate_version(&state, &headers, correlation, None).await {
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
    let key: [u8; 32] = Sha256::digest(
        [
            b"finance.document.cursor/v1".as_slice(),
            &state.cursor_key,
            dossier.as_bytes(),
        ]
        .concat(),
    )
    .into();
    let after = match query.cursor {
        None => Uuid::nil(),
        Some(cursor) => match decode_cursor(&key, state.scope, actor, &cursor, time) {
            Some(id) => id,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    match repository::list(&state.runtime, &proof, dossier, after, limit).await {
        Ok(items) => {
            let next_cursor = if items.len() == limit as usize {
                items
                    .last()
                    .map(|d| encode_cursor(&key, state.scope, actor, d.id, time + 300))
            } else {
                None
            };
            let mut response = Json(Page { items, next_cursor }).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(e) => error(status(e), correlation),
    }
}

pub(super) async fn evidence(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    let proof = match authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(value) => return error(value, correlation),
    };
    match repository::get_evidence(&state.runtime, &proof, id).await {
        Ok(Some(evidence)) => {
            let mut response = Json(evidence).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(value) => error(status(value), correlation),
    }
}
