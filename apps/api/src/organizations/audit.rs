//! Scoped auditor HTTP view over the append-only audit chain.
use super::{OrganizationState, PageQuery, dossiers, error, now, random_id};
use axum::{
    Json,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use finance_shared::{contracts::Principal, ids::CompanyScope};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use std::sync::Arc;
use uuid::Uuid;

type Hmac256 = Hmac<Sha256>;

#[derive(Serialize)]
struct Audit {
    id: Uuid,
    actor_id: Uuid,
    resource_id: Uuid,
    action: String,
    correlation_id: Uuid,
    occurred_at: String,
    digest: String,
}
#[derive(Serialize)]
struct Page {
    items: Vec<Audit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}
fn mac(
    key: &[u8; 32],
    scope: CompanyScope,
    actor: finance_shared::ids::ActorId,
    payload: &[u8],
) -> Hmac256 {
    let mut mac = Hmac256::new_from_slice(key).expect("fixed HMAC key");
    mac.update(b"finance.audit.cursor/v1");
    mac.update(Uuid::from(scope.tenant).as_bytes());
    mac.update(Uuid::from(scope.company).as_bytes());
    mac.update(Uuid::from(actor).as_bytes());
    mac.update(payload);
    mac
}
fn encode_cursor(
    key: &[u8; 32],
    scope: CompanyScope,
    actor: finance_shared::ids::ActorId,
    sequence: i64,
    expires: u64,
) -> String {
    let mut bytes = Vec::with_capacity(48);
    bytes.extend_from_slice(&sequence.to_be_bytes());
    bytes.extend_from_slice(&expires.to_be_bytes());
    bytes.extend_from_slice(&mac(key, scope, actor, &bytes).finalize().into_bytes());
    URL_SAFE_NO_PAD.encode(bytes)
}
fn decode_cursor(
    key: &[u8; 32],
    scope: CompanyScope,
    actor: finance_shared::ids::ActorId,
    value: &str,
    now: u64,
) -> Option<i64> {
    if value.len() > 96 {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(value).ok()?;
    if bytes.len() != 48 {
        return None;
    }
    mac(key, scope, actor, &bytes[..16])
        .verify_slice(&bytes[16..])
        .ok()?;
    let sequence = i64::from_be_bytes(bytes[..8].try_into().ok()?);
    let expires = u64::from_be_bytes(bytes[8..16].try_into().ok()?);
    if sequence < 0 || expires <= now || expires > now.checked_add(300)? {
        return None;
    }
    Some(sequence)
}
fn digest_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

pub(super) async fn list(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
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
    if !(1..=100).contains(&limit) {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let Some(time) = now() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let after = match query.cursor {
        None => 0,
        Some(value) => match decode_cursor(&state.cursor_key, state.scope, actor, &value, time) {
            Some(sequence) => sequence,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    match finance_audit::read_page_authenticated(&state.runtime, &proof, after, limit).await {
        Ok(events) => {
            let next_cursor = if events.len() == limit as usize {
                events.last().map(|event| {
                    encode_cursor(
                        &state.cursor_key,
                        state.scope,
                        actor,
                        event.sequence,
                        time + 300,
                    )
                })
            } else {
                None
            };
            let items = events
                .into_iter()
                .filter_map(|event| {
                    let occurred =
                        chrono::DateTime::<chrono::Utc>::from_timestamp_micros(event.unix_micros)?;
                    Some(Audit {
                        id: event.id,
                        actor_id: event.actor,
                        resource_id: event.resource,
                        action: event.action,
                        correlation_id: event.correlation,
                        occurred_at: occurred.to_rfc3339(),
                        digest: digest_hex(&event.digest),
                    })
                })
                .collect();
            let mut response = Json(Page { items, next_cursor }).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(e) => error(dossiers::status(e), correlation),
    }
}
