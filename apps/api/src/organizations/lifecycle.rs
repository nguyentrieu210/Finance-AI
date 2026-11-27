//! E18 explicit lifecycle HTTP commands. No generic status PATCH.
use super::{OrganizationState, dossiers, error, random_id};
use axum::{
    Json,
    extract::{
        Path, State,
        rejection::JsonRejection,
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use finance_shared::{
    contracts::Version,
    http_contract::{IdempotencyKey, VersionTarget, VersionToken},
};
use finance_workflow::lifecycle;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadyBody {
    check_run_id: Uuid,
}

fn key(headers: &HeaderMap) -> Option<IdempotencyKey> {
    headers
        .get("idempotency-key")
        .and_then(|header| header.to_str().ok())
        .and_then(|value| IdempotencyKey::parse(value).ok())
}

fn dossier_version(headers: &HeaderMap, dossier: Uuid) -> Result<Version, StatusCode> {
    let token = headers
        .get("if-finance-version")
        .and_then(|header| header.to_str().ok())
        .and_then(|value| VersionToken::parse(value).ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    if token.target != VersionTarget::Dossier || Uuid::from(token.resource_id) != dossier {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(token.version)
}

pub(super) async fn ready(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
    body: Result<Json<ReadyBody>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let version = match dossier_version(&headers, dossier) {
        Ok(version) => version,
        Err(status) => return error(status, correlation),
    };
    let proof =
        match dossiers::authenticate_version(&state, &headers, correlation, Some(version)).await {
            Ok(proof) => proof,
            Err(status) => return error(status, correlation),
        };
    let Some(key) = key(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let request = lifecycle::Ready {
        dossier,
        check_run: body.check_run_id,
        snapshot: random_id(),
    };
    match lifecycle::ready(&state.runtime, &proof, &key, request).await {
        Ok(draft) => {
            let token = VersionToken {
                target: VersionTarget::Dossier,
                resource_id: finance_shared::ids::ResourceId::try_from(draft.id)
                    .expect("stored dossier id"),
                version: Version::try_from(draft.version).expect("stored dossier version"),
            };
            let mut response = (StatusCode::OK, Json(draft)).into_response();
            response.headers_mut().insert(
                "finance-version",
                HeaderValue::from_str(&token.to_string()).expect("canonical version"),
            );
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(value) => error(dossiers::status(value), correlation),
    }
}

pub(super) async fn snapshot(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    match lifecycle::snapshot(&state.runtime, &proof, dossier).await {
        Ok(Some(snapshot)) => {
            let mut response = Json(snapshot).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(value) => error(dossiers::status(value), correlation),
    }
}
