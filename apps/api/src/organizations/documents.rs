//! Metadata-only document routes in the injected synthetic router.
use super::dossiers::{authenticate_version, status};
use super::{OrganizationState, decode_cursor, encode_cursor, error, error_with, now, random_id};
use axum::{
    Json,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use finance_documents::repository::{self, Document, Metadata, UploadGrantError};
use finance_identity::AuthenticatedCommand;
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
pub struct FinalizeBody {
    sha256: String,
    generation: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupersedeBody {
    replacement_id: Uuid,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReasonBody {
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifyBody {
    kind: String,
}

async fn authorize_dossier_command(
    state: &OrganizationState,
    headers: &HeaderMap,
    dossier: Uuid,
    correlation: Uuid,
) -> Result<(AuthenticatedCommand, IdempotencyKey), Response> {
    let Some(token) = headers
        .get("if-finance-version")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| VersionToken::parse(s).ok())
    else {
        return Err(error(StatusCode::BAD_REQUEST, correlation));
    };
    if token.target != VersionTarget::Dossier || Uuid::from(token.resource_id) != dossier {
        return Err(error(StatusCode::BAD_REQUEST, correlation));
    }
    let proof = authenticate_version(state, headers, correlation, Some(token.version))
        .await
        .map_err(|s| error(s, correlation))?;
    let Some(key) = headers
        .get("idempotency-key")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| IdempotencyKey::parse(s).ok())
    else {
        return Err(error(StatusCode::BAD_REQUEST, correlation));
    };
    Ok((proof, key))
}

fn dossier_versioned<T: Serialize>(
    code: StatusCode,
    dossier: Uuid,
    version_num: i64,
    payload: T,
) -> Response {
    let token = VersionToken {
        target: VersionTarget::Dossier,
        resource_id: ResourceId::try_from(dossier).expect("stored nonnil dossier id"),
        version: Version::try_from(version_num).expect("stored version"),
    };
    let mut response = (code, Json(payload)).into_response();
    response.headers_mut().insert(
        "finance-version",
        HeaderValue::from_str(&token.to_string()).expect("canonical header"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
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
    let (proof, key) = match authorize_dossier_command(&state, &headers, dossier, correlation).await
    {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let Ok(Json(body)) = body else {
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
            let version = receipt.version;
            dossier_versioned(StatusCode::CREATED, dossier, version, receipt)
        }
        Err(e) => error(status(e), correlation),
    }
}

pub(super) async fn issue_upload_grant(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path((dossier, document)): Path<(Uuid, Uuid)>,
) -> Response {
    let correlation = random_id();
    let (proof, key) = match authorize_dossier_command(&state, &headers, dossier, correlation).await
    {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    match repository::issue_upload_grant(&state.runtime, &proof, &key, dossier, document).await {
        Ok(grant) => {
            let version = grant.dossier_version;
            dossier_versioned(StatusCode::CREATED, dossier, version, grant)
        }
        Err(UploadGrantError::Inactive) => error_with(
            StatusCode::CONFLICT,
            "UPLOAD_GRANT_INACTIVE",
            "Upload grant expired or superseded",
            correlation,
        ),
        Err(UploadGrantError::Core(e)) => error(status(e), correlation),
    }
}

pub(super) async fn finalize(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path((dossier, document)): Path<(Uuid, Uuid)>,
    body: Result<Json<FinalizeBody>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let (proof, key) = match authorize_dossier_command(&state, &headers, dossier, correlation).await
    {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match repository::finalize(
        &state.runtime,
        &proof,
        &key,
        dossier,
        document,
        &body.sha256,
        body.generation,
        random_id(),
    )
    .await
    {
        Ok(job) => {
            let version = job.dossier_version;
            dossier_versioned(StatusCode::ACCEPTED, dossier, version, job)
        }
        Err(e) => error(status(e), correlation),
    }
}

pub(super) async fn supersede(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path((dossier, document)): Path<(Uuid, Uuid)>,
    body: Result<Json<SupersedeBody>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let (proof, key) = match authorize_dossier_command(&state, &headers, dossier, correlation).await
    {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match repository::supersede(
        &state.runtime,
        &proof,
        &key,
        dossier,
        document,
        body.replacement_id,
        &body.reason,
    )
    .await
    {
        Ok(updated) => {
            let version = updated.version;
            dossier_versioned(StatusCode::OK, dossier, version, updated)
        }
        Err(e) => error(status(e), correlation),
    }
}

async fn run_pipeline(
    state: Arc<OrganizationState>,
    headers: HeaderMap,
    dossier: Uuid,
    document: Uuid,
    operation: &str,
    value: Option<String>,
) -> Response {
    let correlation = random_id();
    let (proof, key) = match authorize_dossier_command(&state, &headers, dossier, correlation).await
    {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let Some(value) = value else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match repository::request_pipeline_job(
        &state.runtime,
        &proof,
        &key,
        dossier,
        document,
        operation,
        &value,
        random_id(),
    )
    .await
    {
        Ok(job) => {
            let version = job.dossier_version;
            dossier_versioned(StatusCode::ACCEPTED, dossier, version, job)
        }
        Err(e) => error(status(e), correlation),
    }
}

pub(super) async fn rerun_ocr(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path((dossier, document)): Path<(Uuid, Uuid)>,
    body: Result<Json<ReasonBody>, JsonRejection>,
) -> Response {
    run_pipeline(
        state,
        headers,
        dossier,
        document,
        "document.ocr",
        body.ok().map(|Json(b)| b.reason),
    )
    .await
}

pub(super) async fn classify(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path((dossier, document)): Path<(Uuid, Uuid)>,
    body: Result<Json<ClassifyBody>, JsonRejection>,
) -> Response {
    run_pipeline(
        state,
        headers,
        dossier,
        document,
        "document.classify",
        body.ok().map(|Json(b)| b.kind),
    )
    .await
}

pub(super) async fn extract(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path((dossier, document)): Path<(Uuid, Uuid)>,
    body: Result<Json<ReasonBody>, JsonRejection>,
) -> Response {
    run_pipeline(
        state,
        headers,
        dossier,
        document,
        "document.extract",
        body.ok().map(|Json(b)| b.reason),
    )
    .await
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

#[derive(Serialize)]
struct Download {
    url: String,
    expires_at: String,
    sha256: String,
}

fn download_key(state: &OrganizationState, dossier: Uuid, digest: &str) -> [u8; 32] {
    Sha256::digest(
        [
            b"finance.document.download/v1".as_slice(),
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
    Path((dossier, document)): Path<(Uuid, Uuid)>,
) -> Response {
    let correlation = random_id();
    let proof = match authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(s) => return error(s, correlation),
    };
    let Principal::User(actor) = proof.context().actor else {
        return error(StatusCode::FORBIDDEN, correlation);
    };
    let digest = match repository::original_digest(&state.runtime, &proof, dossier, document).await
    {
        Ok(Some(value)) => value,
        Ok(None) => return error(StatusCode::NOT_FOUND, correlation),
        Err(e) => return error(status(e), correlation),
    };
    let Some(expires) = now().and_then(|value| value.checked_add(120)) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let Some(timestamp) = chrono::DateTime::from_timestamp(expires as i64, 0) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let key = download_key(&state, dossier, &digest);
    let token = encode_cursor(&key, state.scope, actor, document, expires);
    let mut out = Json(Download {
        url: format!("/v1/dossiers/{dossier}/documents/{document}/content?grant={token}"),
        expires_at: timestamp.to_rfc3339(),
        sha256: digest,
    })
    .into_response();
    out.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    out
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ContentGrant {
    grant: String,
}

pub(super) async fn content(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path((dossier, document)): Path<(Uuid, Uuid)>,
    query: Result<Query<ContentGrant>, QueryRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(s) => return error(s, correlation),
    };
    let Principal::User(actor) = proof.context().actor else {
        return error(StatusCode::FORBIDDEN, correlation);
    };
    let Ok(Query(query)) = query else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let digest = match repository::original_digest(&state.runtime, &proof, dossier, document).await
    {
        Ok(Some(value)) => value,
        Ok(None) => return error(StatusCode::NOT_FOUND, correlation),
        Err(e) => return error(status(e), correlation),
    };
    let Some(time) = now() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let key = download_key(&state, dossier, &digest);
    if decode_cursor(&key, state.scope, actor, &query.grant, time) != Some(document) {
        return error(StatusCode::FORBIDDEN, correlation);
    }
    match repository::download_original(&state.runtime, &proof, dossier, document).await {
        Ok(Some(row)) if row.sha256 == digest => {
            let Ok(mime) = HeaderValue::from_str(&row.mime) else {
                return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
            };
            let mut out = row.bytes.into_response();
            out.headers_mut().insert(header::CONTENT_TYPE, mime);
            out.headers_mut().insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment"),
            );
            out.headers_mut().insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            out.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            out
        }
        Ok(_) => error(StatusCode::NOT_FOUND, correlation),
        Err(e) => error(status(e), correlation),
    }
}
