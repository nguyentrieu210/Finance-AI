//! E20 export request endpoint. Artifact rendering is worker-only.
use super::{OrganizationState, dossiers, error, random_id};
use axum::{
    Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use finance_shared::{
    contracts::Version,
    http_contract::{IdempotencyKey, VersionTarget, VersionToken},
    ids::ResourceId,
};
use finance_templates::export;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportBody {
    snapshot_id: Uuid,
    template_ids: Vec<Uuid>,
}
fn key(headers: &HeaderMap) -> Option<IdempotencyKey> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| IdempotencyKey::parse(value).ok())
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
