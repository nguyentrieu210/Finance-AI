//! E21 risk reads and dossier waiver commands.
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
use finance_risk_engine::repository::{self, Risk, Waiver};
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
struct RequestWaiver {
    result_id: Uuid,
    reason: String,
    evidence_ids: Vec<Uuid>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecideWaiver {
    decision: String,
    expires_at: Option<String>,
    reason: String,
    evidence_ids: Vec<Uuid>,
}
fn key(h: &HeaderMap) -> Option<IdempotencyKey> {
    h.get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| IdempotencyKey::parse(s).ok())
}
async fn proof_version(
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
fn waiver_receipt(w: Waiver, status: StatusCode) -> Response {
    let Ok(v) = Version::try_from(w.dossier_version) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, random_id());
    };
    let token = VersionToken {
        target: VersionTarget::Dossier,
        resource_id: ResourceId::try_from(w.dossier_id).expect("stored dossier"),
        version: v,
    };
    let mut out = (status, Json(w)).into_response();
    out.headers_mut().insert(
        "finance-version",
        HeaderValue::from_str(&token.to_string()).expect("canonical"),
    );
    out.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    out
}
pub(super) async fn list_risks(
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
    let ck: [u8; 32] =
        Sha256::digest([b"finance.risk.cursor/v1".as_slice(), &state.cursor_key].concat()).into();
    let after = match q.cursor {
        None => Uuid::nil(),
        Some(x) => match decode_cursor(&ck, state.scope, actor, &x, time) {
            Some(id) => id,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    match repository::list_risks(&state.runtime, &proof, after, limit).await {
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
pub(super) async fn get_risk(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(p) => p,
        Err(s) => return error(s, correlation),
    };
    match repository::get_risk(&state.runtime, &proof, id).await {
        Ok(Some(risk)) => {
            let token = VersionToken {
                target: VersionTarget::Risk,
                resource_id: ResourceId::try_from(risk.id).expect("stored risk"),
                version: Version::try_from(0).expect("zero"),
            };
            let mut out = Json(risk).into_response();
            out.headers_mut().insert(
                "finance-version",
                HeaderValue::from_str(&token.to_string()).expect("canonical"),
            );
            out
        }
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(e) => error(dossiers::status(e), correlation),
    }
}
pub(super) async fn list_waivers(
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
    let Some(time) = now() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let ck: [u8; 32] = Sha256::digest(
        [
            b"finance.waiver.cursor/v1".as_slice(),
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
    match repository::list_waivers(&state.runtime, &proof, dossier, after, limit).await {
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
pub(super) async fn request_waiver(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(dossier): Path<Uuid>,
    body: Result<Json<RequestWaiver>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match proof_version(&state, &headers, correlation, dossier).await {
        Ok(p) => p,
        Err(s) => return error(s, correlation),
    };
    let Some(key) = key(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(b)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match repository::request_waiver(
        &state.runtime,
        &proof,
        &key,
        repository::WaiverRequestCommand {
            dossier,
            id: random_id(),
            result: b.result_id,
            reason: &b.reason,
            evidence: &b.evidence_ids,
        },
    )
    .await
    {
        Ok(w) => waiver_receipt(w, StatusCode::OK),
        Err(e) => error(dossiers::status(e), correlation),
    }
}
pub(super) async fn decide_waiver(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path((dossier, waiver)): Path<(Uuid, Uuid)>,
    body: Result<Json<DecideWaiver>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match proof_version(&state, &headers, correlation, dossier).await {
        Ok(p) => p,
        Err(s) => return error(s, correlation),
    };
    let Some(key) = key(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(b)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let expires = match b
        .expires_at
        .as_deref()
        .map(chrono::DateTime::parse_from_rfc3339)
        .transpose()
    {
        Ok(v) => v.map(|x| x.with_timezone(&chrono::Utc)),
        Err(_) => return error(StatusCode::BAD_REQUEST, correlation),
    };
    match repository::decide_waiver(
        &state.runtime,
        &proof,
        &key,
        repository::WaiverDecisionCommand {
            dossier,
            waiver,
            decision: &b.decision,
            expires,
            reason: &b.reason,
            evidence: &b.evidence_ids,
        },
    )
    .await
    {
        Ok(w) => waiver_receipt(w, StatusCode::OK),
        Err(e) => error(dossiers::status(e), correlation),
    }
}
