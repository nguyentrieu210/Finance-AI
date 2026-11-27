//! E20 export request endpoint. Artifact rendering is worker-only.
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
    contracts::Principal,
    contracts::Version,
    http_contract::{IdempotencyKey, VersionTarget, VersionToken},
    ids::ResourceId,
};
use finance_templates::export;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExportBody {
    snapshot_id: Uuid,
    template_ids: Vec<Uuid>,
}

#[derive(Serialize)]
struct Page {
    items: Vec<export::Package>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}
fn key(headers: &HeaderMap) -> Option<IdempotencyKey> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| IdempotencyKey::parse(value).ok())
}
pub(super) async fn list(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
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
    if dossier.is_nil() || !(1..=100).contains(&limit) {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let Some(time) = now() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let key: [u8; 32] = Sha256::digest(
        [
            b"finance.export.cursor/v1".as_slice(),
            &state.cursor_key,
            dossier.as_bytes(),
        ]
        .concat(),
    )
    .into();
    let after = match query.cursor {
        None => Uuid::nil(),
        Some(value) => match decode_cursor(&key, state.scope, actor, &value, time) {
            Some(value) => value,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    match export::list(&state.runtime, &proof, dossier, after, limit).await {
        Ok(Some(items)) => {
            let next_cursor = if items.len() == limit as usize {
                items
                    .last()
                    .map(|row| encode_cursor(&key, state.scope, actor, row.id, time + 300))
            } else {
                None
            };
            let mut response = Json(Page { items, next_cursor }).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(value) => error(dossiers::status(value), correlation),
    }
}

pub(super) async fn create(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
    body: Result<Json<ExportBody>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let Some(token) = headers
        .get("if-finance-version")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| VersionToken::parse(value).ok())
    else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    if token.target != VersionTarget::Dossier || Uuid::from(token.resource_id) != dossier {
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
    match export::request(
        &state.runtime,
        &proof,
        &key,
        dossier,
        body.snapshot_id,
        &body.template_ids,
        random_id(),
    )
    .await
    {
        Ok(job) => {
            let version = Version::try_from(job.dossier_version).expect("stored dossier version");
            let receipt = VersionToken {
                target: VersionTarget::Dossier,
                resource_id: ResourceId::try_from(dossier).expect("path dossier"),
                version,
            };
            let mut response = (StatusCode::ACCEPTED, Json(job)).into_response();
            response.headers_mut().insert(
                "finance-version",
                HeaderValue::from_str(&receipt.to_string()).expect("canonical"),
            );
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(value) => error(dossiers::status(value), correlation),
    }
}
