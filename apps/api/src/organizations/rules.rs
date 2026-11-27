//! E15 immutable rule draft/create and scoped published/admin reads.
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
use finance_rule_engine::repository::{self, Rule};
use finance_shared::{
    contracts::{Principal, Version},
    http_contract::{IdempotencyKey, VersionTarget, VersionToken},
    ids::ResourceId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    policy_id: Uuid,
    name: String,
    definition: Value,
}
#[derive(Serialize)]
struct RuleResponse {
    id: Uuid,
    policy_id: Uuid,
    version: i64,
    name: String,
    severity: String,
    waivable: bool,
    clause_id: Uuid,
    definition: Value,
}
#[derive(Serialize)]
struct Page {
    items: Vec<RuleResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}
fn response(rule: Rule) -> Result<RuleResponse, ()> {
    Ok(RuleResponse {
        id: rule.id,
        policy_id: rule.policy_id,
        version: rule.version,
        name: rule.name,
        severity: rule.severity,
        waivable: rule.waivable,
        clause_id: rule.clause_id,
        definition: serde_json::from_str(&rule.definition_json).map_err(|_| ())?,
    })
}
fn receipt(rule: Rule, status: StatusCode) -> Response {
    let id = rule.id;
    let version = rule.version;
    let Ok(body) = response(rule) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, random_id());
    };
    let Ok(version) = Version::try_from(version) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, random_id());
    };
    let token = VersionToken {
        target: VersionTarget::Rule,
        resource_id: ResourceId::try_from(id).expect("stored nonnil rule"),
        version,
    };
    let mut out = (status, Json(body)).into_response();
    out.headers_mut().insert(
        "finance-version",
        HeaderValue::from_str(&token.to_string()).expect("canonical token"),
    );
    out.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    out
}
fn key(headers: &HeaderMap) -> Option<IdempotencyKey> {
    headers
        .get("idempotency-key")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| IdempotencyKey::parse(s).ok())
}
pub(super) async fn create(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    body: Result<Json<Create>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(p) => p,
        Err(s) => return error(s, correlation),
    };
    let Some(key) = key(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    match repository::create(
        &state.runtime,
        &proof,
        &key,
        body.policy_id,
        random_id(),
        &body.name,
        &body.definition,
    )
    .await
    {
        Ok(rule) => receipt(rule, StatusCode::CREATED),
        Err(e) => error(dossiers::status(e), correlation),
    }
}
pub(super) async fn get(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(p) => p,
        Err(s) => return error(s, correlation),
    };
    match repository::get(&state.runtime, &proof, id).await {
        Ok(Some(rule)) => receipt(rule, StatusCode::OK),
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(e) => error(dossiers::status(e), correlation),
    }
}
pub(super) async fn list(
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
    let cursor_key: [u8; 32] =
        Sha256::digest([b"finance.rule.cursor/v1".as_slice(), &state.cursor_key].concat()).into();
    let after = match query.cursor {
        None => Uuid::nil(),
        Some(value) => match decode_cursor(&cursor_key, state.scope, actor, &value, time) {
            Some(id) => id,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    match repository::list(&state.runtime, &proof, after, limit).await {
        Ok(rows) => {
            let next_cursor = if rows.len() == limit as usize {
                rows.last()
                    .map(|x| encode_cursor(&cursor_key, state.scope, actor, x.id, time + 300))
            } else {
                None
            };
            let items = match rows
                .into_iter()
                .map(response)
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(v) => v,
                Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, correlation),
            };
            let mut out = Json(Page { items, next_cursor }).into_response();
            out.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            out
        }
        Err(e) => error(dossiers::status(e), correlation),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Revise {
    name: String,
    definition: Value,
    reason: String,
}

pub(super) async fn revise(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: Result<Json<Revise>, JsonRejection>,
) -> Response {
    let correlation = random_id();
    let Some(token) = headers
        .get("if-finance-version")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| VersionToken::parse(s).ok())
    else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    if token.target != VersionTarget::Rule || Uuid::from(token.resource_id) != id {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let proof =
        match dossiers::authenticate_version(&state, &headers, correlation, Some(token.version))
            .await
        {
            Ok(proof) => proof,
            Err(s) => return error(s, correlation),
        };
    let Some(key) = key(&headers) else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(Json(body)) = body else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let draft = repository::ReviseDraft {
        rule: id,
        row: random_id(),
        name: &body.name,
        definition: &body.definition,
        reason: &body.reason,
    };
    match repository::revise(&state.runtime, &proof, &key, draft).await {
        Ok(rule) => receipt(rule, StatusCode::OK),
        Err(e) => error(dossiers::status(e), correlation),
    }
}
