//! E01 authenticated job receipt backed by the durable queue.
use super::{OrganizationState, dossiers, error, random_id};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use finance_core::prototype::{recheck_authenticated, scoped_transaction};
use finance_permissions::lock_roles;
use finance_shared::contracts::Principal;
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Serialize, sqlx::FromRow)]
struct Job {
    id: Uuid,
    state: String,
    attempt: i64,
    progress: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
}

pub(super) async fn get(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    if id.is_nil() {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    let Principal::User(actor) = proof.context().actor else {
        return error(StatusCode::FORBIDDEN, correlation);
    };
    let mut tx = match scoped_transaction(&state.runtime, proof.context()).await {
        Ok(tx) => tx,
        Err(value) => return error(dossiers::status(value), correlation),
    };
    match lock_roles(&mut tx, proof.context()).await {
        Ok(roles) if !roles.is_empty() => {}
        Ok(_) | Err(finance_permissions::GrantError::Forbidden) => {
            return error(StatusCode::FORBIDDEN, correlation);
        }
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, correlation),
    }
    if let Err(value) = recheck_authenticated(&mut tx, Some(&proof)).await {
        return error(dossiers::status(value), correlation);
    }
    let row = sqlx::query_as::<_, Job>("SELECT * FROM finance.runtime_job_status($1,$2,$3,$4)")
        .bind(Uuid::from(state.scope.tenant))
        .bind(Uuid::from(state.scope.company))
        .bind(Uuid::from(actor))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await;
    let job = match row {
        Ok(Some(job)) => job,
        Ok(None) => return error(StatusCode::NOT_FOUND, correlation),
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, correlation),
    };
    if let Err(value) = recheck_authenticated(&mut tx, Some(&proof)).await {
        return error(dossiers::status(value), correlation);
    }
    if tx.commit().await.is_err() {
        return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
    }
    let mut response = Json(job).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
