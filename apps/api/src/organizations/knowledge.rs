//! Synthetic-only E17 operational diagnostic. Not part of production OpenAPI.
use super::{OrganizationState, dossiers, error, random_id};
use axum::{
    Json,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticQuery {
    kind: String,
    day: String,
    q: String,
    limit: Option<i64>,
}

pub(super) async fn search(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    query: Result<Query<DiagnosticQuery>, QueryRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    let Ok(Query(query)) = query else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let Ok(day) = NaiveDate::parse_from_str(&query.day, "%Y-%m-%d") else {
        return error(StatusCode::BAD_REQUEST, correlation);
    };
    let model = "3333333333333333333333333333333333333333333333333333333333333333";
    let request = finance_knowledge::repository::Retrieval {
        kind: &query.kind,
        day,
        query: &query.q,
        model_sha256: model,
        vector: [1.0, 0.0, 0.0],
        limit: query.limit.unwrap_or(10),
    };
    match finance_knowledge::repository::retrieve(&state.runtime, &proof, request).await
    {
        Ok(items) => {
            let mut response = Json(items).into_response();
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(e) => error(dossiers::status(e), correlation),
    }
}
