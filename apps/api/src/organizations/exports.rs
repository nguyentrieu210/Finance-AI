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

#[derive(Serialize)]
struct Download {
    url: String,
    expires_at: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ContentGrant {
    grant: String,
}

#[derive(Serialize)]
struct ExportManifest {
    package_id: Uuid,
    dossier_id: Uuid,
    snapshot_id: Uuid,
    template_id: Uuid,
    sha256: String,
    artifact_count: i32,
    artifact_bytes: i64,
}

fn export_download_key(state: &OrganizationState, dossier: Uuid, digest: &str) -> [u8; 32] {
    Sha256::digest(
        [
            b"finance.export.download/v1".as_slice(),
            &state.cursor_key,
            dossier.as_bytes(),
            digest.as_bytes(),
        ]
        .concat(),
    )
    .into()
}

pub(super) async fn download(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path((dossier, package)): Path<(Uuid, Uuid)>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    let Principal::User(actor) = proof.context().actor else {
        return error(StatusCode::FORBIDDEN, correlation);
    };
    let digest = match export::export_manifest(&state.runtime, &proof, dossier, package).await {
        Ok(Some(value)) => value,
        Ok(None) => return error(StatusCode::NOT_FOUND, correlation),
        Err(value) => return error(dossiers::status(value), correlation),
    };
    let Some(expires) = now().and_then(|value| value.checked_add(120)) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let Some(timestamp) = chrono::DateTime::from_timestamp(expires as i64, 0) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let key = export_download_key(&state, dossier, &digest);
    let token = encode_cursor(&key, state.scope, actor, package, expires);
    let mut response = Json(Download {
        url: format!("/v1/dossiers/{dossier}/exports/{package}/content?grant={token}"),
        expires_at: timestamp.to_rfc3339(),
        sha256: digest,
    })
    .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(super) async fn content(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path((dossier, package)): Path<(Uuid, Uuid)>,
    query: Result<Query<ContentGrant>, QueryRejection>,
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
    let digest = match export::export_manifest(&state.runtime, &proof, dossier, package).await {
        Ok(Some(value)) => value,
        Ok(None) => return error(StatusCode::NOT_FOUND, correlation),
        Err(value) => return error(dossiers::status(value), correlation),
    };
    let Some(time) = now() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let key = export_download_key(&state, dossier, &digest);
    if decode_cursor(&key, state.scope, actor, &query.grant, time) != Some(package) {
        return error(StatusCode::FORBIDDEN, correlation);
    }
    match export::download_export(&state.runtime, &proof, dossier, package).await {
        Ok(Some(row)) if row.sha256 == digest => {
            match export::bundle(&state.runtime,&proof,dossier,package).await {
                Ok(Some(bundle))=>{
                    let Some(root)=std::env::var_os("FINANCE_EXPORT_ARTIFACT_ROOT") else {return error(StatusCode::SERVICE_UNAVAILABLE,correlation)};
                    let scope=state.scope;
                    let digest=bundle.sha256.clone();
                    let bytes=tokio::task::spawn_blocking(move||export::read_bundle(std::path::Path::new(&root),scope,&bundle)).await;
                    let Ok(Some(bytes))=bytes else{return error(StatusCode::SERVICE_UNAVAILABLE,correlation)};
                    let Ok(sha)=HeaderValue::from_str(&digest) else{return error(StatusCode::SERVICE_UNAVAILABLE,correlation)};
                    return ([(header::CONTENT_TYPE,HeaderValue::from_static("application/zip")),
                    (header::CONTENT_DISPOSITION,HeaderValue::from_static("attachment; filename=finance-package.zip")),
                    (header::CACHE_CONTROL,HeaderValue::from_static("no-store")),
                    (header::X_CONTENT_TYPE_OPTIONS,HeaderValue::from_static("nosniff")),
                    (axum::http::HeaderName::from_static("x-content-sha256"),sha)],bytes).into_response();
                }
                Ok(None)=>{},
                Err(value)=>return error(dossiers::status(value),correlation),
            }
            let sha_header = match HeaderValue::from_str(&row.sha256) {
                Ok(value) => value,
                Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, correlation),
            };
            let manifest = ExportManifest {
                package_id: row.package_id,
                dossier_id: dossier,
                snapshot_id: row.snapshot_id,
                template_id: row.template_id,
                sha256: row.sha256,
                artifact_count: row.artifact_count,
                artifact_bytes: row.artifact_bytes,
            };
            let mut response = Json(manifest).into_response();
            response.headers_mut().insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment"),
            );
            response.headers_mut().insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            response
                .headers_mut()
                .insert("x-content-sha256", sha_header);
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Ok(_) => error(StatusCode::NOT_FOUND, correlation),
        Err(value) => error(dossiers::status(value), correlation),
    }
}
