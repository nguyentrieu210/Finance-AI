//! Approved reference list endpoints for contracts, advances and budgets.
use super::{OrganizationState, PageQuery, decode_cursor, dossiers, encode_cursor, error, now, random_id};
use axum::{
    Json,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use finance_financial::references::Reference;
use finance_shared::contracts::Principal;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Serialize)]
struct Page {
    items: Vec<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Contract,
    Advance,
    Budget,
}
impl Kind {
    fn key(self) -> &'static [u8] {
        match self {
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
        Kind::Contract => finance_contracts::list_verified(&state.runtime, &proof, after, limit).await,
        Kind::Advance => finance_advances::list_verified(&state.runtime, &proof, after, limit).await,
        Kind::Budget => finance_budget::list_verified(&state.runtime, &proof, after, limit).await,
    };
    match result {
        Ok(items) => {
            let next_cursor = if items.len() == limit as usize {
                items.last().map(|row| encode_cursor(&key, state.scope, actor, row.id, time + 300))
            } else {
                None
            };
            let mut response = Json(Page { items, next_cursor }).into_response();
            response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
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
