//! Synthetic draft API, mounted only on the explicitly injected test router.
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
use finance_core::prototype::Error;
use finance_identity::{AuthenticatedCommand, IdentityError};
use finance_shared::{
    contracts::{Principal, Version},
    http_contract::{IdempotencyKey, VersionTarget, VersionToken},
    ids::{CommandId, CorrelationId, ResourceId},
};
use finance_workflow::draft::{self, Draft, Kind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DossierKind {
    Payment,
    Advance,
    Settlement,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    company_id: Uuid,
    kind: DossierKind,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct List {
    limit: Option<i64>,
    cursor: Option<String>,
    company_id: Option<Uuid>,
    kind: Option<String>,
    status: Option<String>,
    updated_from: Option<String>,
    updated_to: Option<String>,
}

#[derive(Serialize)]
struct Page {
    items: Vec<Draft>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

pub(super) async fn authenticate_version(
    state: &OrganizationState,
    headers: &HeaderMap,
    correlation: Uuid,
    version: Option<Version>,
) -> Result<AuthenticatedCommand, StatusCode> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .filter(|v| !v.is_empty() && v.len() <= 16384)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    state
        .verifier
        .authenticate_command(
            &state.lookup,
            token,
            state.scope,
            CommandId::try_from(random_id()).expect("random command"),
            CorrelationId::try_from(correlation).expect("random correlation"),
            version,
        )
        .await
        .map_err(|e| match e {
            IdentityError::InvalidToken => StatusCode::UNAUTHORIZED,
            IdentityError::Forbidden => StatusCode::FORBIDDEN,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })
}

pub(super) fn status(e: Error) -> StatusCode {
    match e {
        Error::Forbidden => StatusCode::FORBIDDEN,
        Error::InvalidInput => StatusCode::BAD_REQUEST,
        Error::Conflict => StatusCode::CONFLICT,
        Error::Database(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

fn receipt(draft: Draft, status: StatusCode) -> Response {
    let token = VersionToken {
        target: VersionTarget::Dossier,
        resource_id: ResourceId::try_from(draft.id).expect("stored nonnil ID"),
        version: Version::try_from(draft.version).expect("stored version"),
    };
    let mut response = (status, Json(draft)).into_response();
    response.headers_mut().insert(
        "finance-version",
        HeaderValue::from_str(&token.to_string()).expect("canonical header"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub async fn create(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    body: Result<Json<Create>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match authenticate_version(&state, &headers, correlation, None).await {
        Ok(p) => p,
        Err(s) => return error(s, correlation),
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    if body.company_id != Uuid::from(state.scope.company) {
        return error(StatusCode::FORBIDDEN, correlation);
    }
    let Some(key) = headers
        .get("idempotency-key")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| IdempotencyKey::parse(s).ok())
    else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let kind = match body.kind {
        DossierKind::Payment => Kind::Payment,
        DossierKind::Advance => Kind::Advance,
        DossierKind::Settlement => Kind::Settlement,
    };
    match draft::create(&state.runtime, &proof, &key, kind, random_id(), random_id()).await {
        Ok(d) => receipt(d, StatusCode::CREATED),
        Err(e) => error(status(e), correlation),
    }
}

pub async fn get(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    let proof = match authenticate_version(&state, &headers, correlation, None).await {
        Ok(p) => p,
        Err(s) => return error(s, correlation),
    };
    match draft::get(&state.runtime, &proof, id).await {
        Ok(Some(d)) => receipt(d, StatusCode::OK),
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(e) => error(status(e), correlation),
    }
}

pub async fn list(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
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
    let parse_date = |value: Option<&str>| {
        value
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(s).map(|date| date.with_timezone(&chrono::Utc))
            })
            .transpose()
    };
    let (Ok(updated_from), Ok(updated_to)) = (
        parse_date(query.updated_from.as_deref()),
        parse_date(query.updated_to.as_deref()),
    ) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let filters = draft::Filters {
        company_id: query.company_id,
        kind: query.kind.as_deref(),
        status: query.status.as_deref(),
        updated_from,
        updated_to,
    };
    if !filters.valid() {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    if filters
        .company_id
        .is_some_and(|id| id != Uuid::from(state.scope.company))
    {
        return error(StatusCode::FORBIDDEN, correlation);
    }
    // Bind cursors to the exact filter selection in addition to actor/scope.
    let filter_key = format!(
        "{:?}/{:?}/{:?}/{:?}/{:?}",
        filters.company_id, filters.kind, filters.status, filters.updated_from, filters.updated_to
    );
    let key: [u8; 32] = Sha256::digest(
        [
            b"finance.dossier.cursor/v2".as_slice(),
            &state.cursor_key,
            filter_key.as_bytes(),
        ]
        .concat(),
    )
    .into();
    let after = match query.cursor {
        None => Uuid::nil(),
        Some(value) => match decode_cursor(&key, state.scope, actor, &value, time) {
            Some(id) => id,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    match draft::list_filtered(&state.runtime, &proof, after, limit, &filters).await {
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChangeBody {
    reason: String,
    evidence_ids: Vec<Uuid>,
}

async fn change_dossier(
    state: Arc<OrganizationState>,
    headers: HeaderMap,
    dossier: Uuid,
    body: Result<Json<ChangeBody>, JsonRejection>,
    change: draft::Change,
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
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let input = draft::ChangeInput {
        dossier,
        revision: random_id(),
        change,
        reason: &body.reason,
        evidence: &body.evidence_ids,
    };
    match draft::change(&state.runtime, &proof, &key, input).await {
        Ok(row) => receipt(row, StatusCode::OK),
        Err(e) => error(status(e), correlation),
    }
}

pub(super) async fn revise(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
    body: Result<Json<ChangeBody>, JsonRejection>,
) -> Response {
    change_dossier(state, headers, dossier, body, draft::Change::Revise).await
}

pub(super) async fn reopen(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
    body: Result<Json<ChangeBody>, JsonRejection>,
) -> Response {
    change_dossier(state, headers, dossier, body, draft::Change::Reopen).await
}

pub(super) async fn cancel(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
    body: Result<Json<ChangeBody>, JsonRejection>,
) -> Response {
    change_dossier(state, headers, dossier, body, draft::Change::Cancel).await
}
