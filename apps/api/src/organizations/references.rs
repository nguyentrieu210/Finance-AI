//! Approved and manual reference endpoints for contracts, advances, budgets and payment refs.
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
use finance_financial::references::{CreateReference, Reference, ReferenceReceipt};
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
pub(super) struct CreateBody {
    document_id: Uuid,
    kind: String,
    as_of: String,
    coverage: String,
    fresh_until: Option<String>,
    #[serde(default)]
    records: Vec<ImportRow>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NormalizedResource {
    source_key: String,
    currency: String,
    cap: String,
    consumed: String,
    evidence_ids: Vec<Uuid>,
    details: serde_json::Value,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PaymentRow {
    source_key: String,
    currency: String,
    amount: String,
    reservation_id: Uuid,
    event_kind: String,
    reverses: Option<String>,
    occurred_at: String,
    evidence_ids: Vec<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    allocations: Option<std::collections::BTreeMap<Uuid, String>>,
}
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum ImportRow {
    Resource(NormalizedResource),
    Payment(PaymentRow),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VerifyBody {
    reason: String,
    evidence_ids: Vec<Uuid>,
}

#[derive(Serialize)]
struct Page {
    items: Vec<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    All,
    Contract,
    Advance,
    Budget,
}
impl Kind {
    fn key(self) -> &'static [u8] {
        match self {
            Self::All => b"all",
            Self::Contract => b"contract",
            Self::Advance => b"advance",
            Self::Budget => b"budget",
        }
    }
}

async fn list_kind(
    state: Arc<OrganizationState>,
    headers: HeaderMap,
    query: Result<Query<PageQuery>, QueryRejection>,
    kind: Kind,
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
    let key: [u8; 32] = Sha256::digest(
        [
            b"finance.reference.cursor/v1".as_slice(),
            &state.cursor_key,
            kind.key(),
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
    let result = match kind {
        Kind::All => {
            finance_financial::references::list_all(&state.runtime, &proof, after, limit).await
        }
        Kind::Contract => {
            finance_contracts::list_verified(&state.runtime, &proof, after, limit).await
        }
        Kind::Advance => {
            finance_advances::list_verified(&state.runtime, &proof, after, limit).await
        }
        Kind::Budget => finance_budget::list_verified(&state.runtime, &proof, after, limit).await,
    };
    match result {
        Ok(items) => {
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
        Err(e) => error(dossiers::status(e), correlation),
    }
}

pub(super) async fn contracts(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Response {
    list_kind(state, headers, query, Kind::Contract).await
}
pub(super) async fn advances(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Response {
    list_kind(state, headers, query, Kind::Advance).await
}
pub(super) async fn budgets(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Response {
    list_kind(state, headers, query, Kind::Budget).await
}

pub(super) async fn all(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Response {
    list_kind(state, headers, query, Kind::All).await
}

fn receipt(reference: ReferenceReceipt) -> Response {
    let token = VersionToken {
        target: VersionTarget::Reference,
        resource_id: ResourceId::try_from(reference.id).expect("stored reference"),
        version: Version::try_from(reference.version).expect("stored reference version"),
    };
    let mut response = Json(reference).into_response();
    response.headers_mut().insert(
        "finance-version",
        HeaderValue::from_str(&token.to_string()).expect("canonical"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
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
    match finance_financial::references::get(&state.runtime, &proof, id).await {
        Ok(Some(reference)) => receipt(reference),
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(value) => error(dossiers::status(value), correlation),
    }
}

pub(super) async fn create(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    body: Result<Json<CreateBody>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    let Some(key) = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| IdempotencyKey::parse(value).ok())
    else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(parsed_as_of) = chrono::DateTime::parse_from_rfc3339(&body.as_of) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let as_of = parsed_as_of.with_timezone(&chrono::Utc).to_rfc3339();
    let fresh_until = match body.fresh_until.as_deref() {
        Some(raw) => match chrono::DateTime::parse_from_rfc3339(raw) {
            Ok(value) if value > parsed_as_of => value.to_rfc3339(),
            _ => return error(StatusCode::BAD_REQUEST, correlation),
        },
        None if body.records.is_empty() => {
            (parsed_as_of + chrono::TimeDelta::seconds(1)).to_rfc3339()
        }
        None => return error(StatusCode::BAD_REQUEST, correlation),
    };
    if body
        .records
        .iter()
        .any(|row| matches!(row, ImportRow::Payment(_)) != (body.kind == "PAYMENT"))
    {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    if body.records.len() > 10000 {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let Ok(records) = serde_json::to_value(&body.records) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match finance_financial::references::create(
        &state.runtime,
        &proof,
        &key,
        CreateReference {
            document_id: body.document_id,
            kind: &body.kind,
            as_of: &as_of,
            coverage: &body.coverage,
            fresh_until: &fresh_until,
            records: &records,
            reference_id: random_id(),
        },
    )
    .await
    {
        Ok(job) => {
            let mut response = (StatusCode::ACCEPTED, Json(job)).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(value) => error(dossiers::status(value), correlation),
    }
}

pub(super) async fn verify(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: Result<Json<VerifyBody>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let Some(token) = headers
        .get("if-finance-version")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| VersionToken::parse(value).ok())
    else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    if token.target != VersionTarget::Reference || Uuid::from(token.resource_id) != id {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let proof =
        match dossiers::authenticate_version(&state, &headers, correlation, Some(token.version))
            .await
        {
            Ok(proof) => proof,
            Err(status) => return error(status, correlation),
        };
    let Some(key) = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| IdempotencyKey::parse(value).ok())
    else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match finance_financial::references::verify(
        &state.runtime,
        &proof,
        &key,
        id,
        &body.reason,
        &body.evidence_ids,
    )
    .await
    {
        Ok(reference) => receipt(reference),
        Err(value) => error(dossiers::status(value), correlation),
    }
}

pub(super) async fn material(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(p) => p,
        Err(s) => return error(s, correlation),
    };
    match finance_financial::references::import_material(&state.runtime, &proof, id).await {
        Ok(Some(value)) => {
            let mut response = Json(value).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(e) => error(dossiers::status(e), correlation),
    }
}
pub(super) async fn reservable(
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
    let key: [u8; 32] = Sha256::digest(
        [
            b"finance.reservable.cursor/v1".as_slice(),
            &state.cursor_key,
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
    match finance_financial::references::reservable(&state.runtime, &proof, after, limit).await {
        Ok(items) => {
            let next = if items.len() == limit as usize {
                items
                    .last()
                    .map(|row| encode_cursor(&key, state.scope, actor, row.id, time + 300))
            } else {
                None
            };
            let mut response =
                Json(serde_json::json!({"items":items,"next_cursor":next})).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(e) => error(dossiers::status(e), correlation),
    }
}
