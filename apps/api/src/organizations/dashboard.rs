//! Requester-scoped current-state operational dashboard.
use super::{OrganizationState, PageQuery, dossiers, error, now, random_id};
use axum::{
    Json,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use std::sync::Arc;

#[derive(serde::Serialize)]
struct Page {
    items: Vec<finance_analytics::dashboard::Metric>,
}

pub(super) async fn get(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    let Ok(Query(query)) = query else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    if query.cursor.is_some() {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let limit = query.limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let counts = match finance_workflow::analytics::state_counts(&state.runtime, &proof).await {
        Ok(counts) => counts,
        Err(e) => return error(dossiers::status(e), correlation),
    };
    let Some(epoch) = now() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let Some(as_of) = chrono::DateTime::<chrono::Utc>::from_timestamp(epoch as i64, 0) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    };
    let as_of = as_of.to_rfc3339();
    let mut items = finance_analytics::dashboard::from_state_counts(counts, &as_of);
    items.truncate(limit as usize);
    let mut response = Json(Page { items }).into_response();
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
