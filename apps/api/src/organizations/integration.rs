//! E23 manual handoff and reconciliation HTTP surface. DO_API is intentionally disabled.
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
use finance_integration::repository::{self, Handoff, Reconciliation};
use finance_shared::{
    contracts::{Principal, Version},
    http_contract::{IdempotencyKey, VersionTarget, VersionToken},
    ids::ResourceId,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[derive(serde::Serialize)]
struct Page<T> {
    items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HandoffBody {
    package_id: Uuid,
    channel: String,
    external_ref: Option<String>,
    evidence_ids: Vec<Uuid>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResolveBody {
    reason: String,
    evidence_ids: Vec<Uuid>,
}
fn key(h: &HeaderMap) -> Option<IdempotencyKey> {
    h.get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| IdempotencyKey::parse(s).ok())
}
async fn dossier_proof(
    state: &OrganizationState,
    h: &HeaderMap,
    correlation: Uuid,
    dossier: Uuid,
) -> Result<finance_identity::AuthenticatedCommand, StatusCode> {
    let Some(token) = h
        .get("if-finance-version")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| VersionToken::parse(s).ok())
    else {
        return Err(StatusCode::BAD_REQUEST);
    };
    if token.target != VersionTarget::Dossier || Uuid::from(token.resource_id) != dossier {
        return Err(StatusCode::BAD_REQUEST);
    }
    dossiers::authenticate_version(state, h, correlation, Some(token.version)).await
}
fn handoff_receipt(h: Handoff, status: StatusCode) -> Response {
    let Ok(version) = Version::try_from(h.dossier_version) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, random_id());
    };
    let token = VersionToken {
        target: VersionTarget::Dossier,
        resource_id: ResourceId::try_from(h.dossier_id).expect("stored dossier"),
        version,
    };
    let mut out = (status, Json(h)).into_response();
    out.headers_mut().insert(
        "finance-version",
        HeaderValue::from_str(&token.to_string()).expect("canonical"),
    );
    out.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    out
}
fn reconciliation_receipt(r: Reconciliation, status: StatusCode) -> Response {
    let Ok(version) = Version::try_from(r.version) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, random_id());
    };
    let token = VersionToken {
        target: VersionTarget::Reconciliation,
        resource_id: ResourceId::try_from(r.id).expect("stored reconciliation"),
        version,
    };
    let mut out = (status, Json(r)).into_response();
    out.headers_mut().insert(
        "finance-version",
        HeaderValue::from_str(&token.to_string()).expect("canonical"),
    );
    out.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    out
}
pub(super) async fn list_handoffs(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
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
    let Ok(Query(q)) = query else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let limit = q.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let Some(time) = now() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let ck: [u8; 32] = Sha256::digest(
        [
            b"finance.handoff.cursor/v1".as_slice(),
            &state.cursor_key,
            dossier.as_bytes(),
        ]
        .concat(),
    )
    .into();
    let after = match q.cursor {
        None => Uuid::nil(),
        Some(x) => match decode_cursor(&ck, state.scope, actor, &x, time) {
            Some(id) => id,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    match repository::list_handoffs(&state.runtime, &proof, dossier, after, limit).await {
        Ok(items) => {
            let next_cursor = if items.len() == limit as usize {
                items
                    .last()
                    .map(|x| encode_cursor(&ck, state.scope, actor, x.id, time + 300))
            } else {
                None
            };
            Json(Page { items, next_cursor }).into_response()
        }
        Err(e) => error(dossiers::status(e), correlation),
    }
}
pub(super) async fn create_handoff(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
    body: Result<Json<HandoffBody>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match dossier_proof(&state, &headers, correlation, dossier).await {
        Ok(p) => p,
        Err(s) => return error(s, correlation),
    };
    let Some(key) = key(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    if body.channel != "MANUAL" {
        return error(StatusCode::CONFLICT, correlation);
    }
    let Some(external) = body.external_ref.as_deref() else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match repository::create_manual_handoff(
        &state.runtime,
        &proof,
        &key,
        repository::ManualHandoffCommand {
            dossier,
            handoff: random_id(),
            package: body.package_id,
            external_ref: external,
            evidence: &body.evidence_ids,
        },
    )
    .await
    {
        Ok(h) => handoff_receipt(h, StatusCode::CREATED),
        Err(e) => error(dossiers::status(e), correlation),
    }
}
pub(super) async fn list_reconciliation(
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
    let Ok(Query(q)) = query else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let limit = q.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let Some(time) = now() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let ck: [u8; 32] = Sha256::digest(
        [
            b"finance.reconciliation.cursor/v1".as_slice(),
            &state.cursor_key,
        ]
        .concat(),
    )
    .into();
    let after = match q.cursor {
        None => Uuid::nil(),
        Some(x) => match decode_cursor(&ck, state.scope, actor, &x, time) {
            Some(id) => id,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    match repository::list_reconciliations(&state.runtime, &proof, after, limit).await {
        Ok(items) => {
            let next_cursor = if items.len() == limit as usize {
                items
                    .last()
                    .map(|x| encode_cursor(&ck, state.scope, actor, x.id, time + 300))
            } else {
                None
            };
            Json(Page { items, next_cursor }).into_response()
        }
        Err(e) => error(dossiers::status(e), correlation),
    }
}
pub(super) async fn get_reconciliation(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(p) => p,
        Err(s) => return error(s, correlation),
    };
    match repository::get_reconciliation(&state.runtime, &proof, id).await {
        Ok(Some(r)) => reconciliation_receipt(r, StatusCode::OK),
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(e) => error(dossiers::status(e), correlation),
    }
}
pub(super) async fn resolve_reconciliation(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: Result<Json<ResolveBody>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let Some(token) = headers
        .get("if-finance-version")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| VersionToken::parse(s).ok())
    else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    if token.target != VersionTarget::Reconciliation || Uuid::from(token.resource_id) != id {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let proof =
        match dossiers::authenticate_version(&state, &headers, correlation, Some(token.version))
            .await
        {
            Ok(p) => p,
            Err(s) => return error(s, correlation),
        };
    let Some(key) = key(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match repository::resolve_reconciliation(
        &state.runtime,
        &proof,
        &key,
        id,
        &body.reason,
        &body.evidence_ids,
    )
    .await
    {
        Ok(r) => reconciliation_receipt(r, StatusCode::OK),
        Err(e) => error(dossiers::status(e), correlation),
    }
}

pub(super) async fn payment_status(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(s) => return error(s, correlation),
    };
    match repository::payment_status(&state.runtime, &proof, dossier).await {
        Ok(Some(row)) => {
            let mut response = Json(row).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(e) => error(dossiers::status(e), correlation),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PaymentObservationBody {
    reference_id: Uuid,
    evidence_ids: Vec<Uuid>,
    reason: String,
}

pub(super) async fn record_payment_observation(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
    body: Result<Json<PaymentObservationBody>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let Some(token) = headers
        .get("if-finance-version")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| VersionToken::parse(s).ok())
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
            Ok(p) => p,
            Err(s) => return error(s, correlation),
        };
    let Some(key) = key(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match repository::record_payment_observation(
        &state.runtime,
        &proof,
        &key,
        dossier,
        body.reference_id,
        &body.evidence_ids,
        &body.reason,
    )
    .await
    {
        Ok(row) => {
            let version = Version::try_from(row.dossier_version).expect("stored dossier version");
            let receipt = VersionToken {
                target: VersionTarget::Dossier,
                resource_id: ResourceId::try_from(dossier).expect("path dossier"),
                version,
            };
            let mut response = (StatusCode::OK, Json(row)).into_response();
            response.headers_mut().insert(
                "finance-version",
                HeaderValue::from_str(&receipt.to_string()).expect("canonical"),
            );
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(e) => error(dossiers::status(e), correlation),
    }
}
