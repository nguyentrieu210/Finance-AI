//! Administrative configuration, not a source of financial authorization yet.
use super::{OrganizationState, dossiers, error, random_id};
use axum::{
    Json,
    extract::{Path, Query, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_permissions::{GrantError, Role, require_role};
use finance_shared::{contracts::Principal, http_contract::IdempotencyKey, ids::ResourceScope};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

fn kind_allowed(kind: &str) -> bool {
    matches!(
        kind,
        "positions" | "departments" | "invitations" | "roles" | "assignments"
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    after: Option<Uuid>,
    limit: Option<i32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Save {
    id: Uuid,
    expected_version: Option<i64>,
    payload: Value,
}
fn payload_valid(kind: &str, expected: Option<i64>, payload: &Value) -> bool {
    let Some(obj) = payload.as_object() else {
        return false;
    };
    let allowed: &[&str] = match kind {
        "positions" => &["name", "active"],
        "departments" => &[
            "actor_id",
            "department_id",
            "position_id",
            "is_primary",
            "active",
        ],
        "invitations" if expected.is_none() => &["email"],
        "invitations" => &[],
        "roles" => &["name", "active", "capabilities"],
        "assignments" if expected.is_none() => &[
            "actor_id",
            "department_id",
            "role_id",
            "role_version",
            "valid_until",
        ],
        "assignments" => &[],
        _ => return false,
    };
    obj.keys().all(|k| allowed.contains(&k.as_str())) && payload.to_string().len() <= 4096
}
fn grant(e: GrantError) -> Error {
    match e {
        GrantError::Forbidden => Error::Forbidden,
        GrantError::Database(e) => Error::Database(e),
    }
}
fn db_error(e: sqlx::Error) -> Error {
    match e.as_database_error().and_then(|d| d.code()).as_deref() {
        Some("42501") => Error::Forbidden,
        Some("23514" | "23503" | "23505") => Error::Conflict,
        Some("22023" | "22P02" | "23502" | "22007" | "22008") => Error::InvalidInput,
        _ => Error::Database(e),
    }
}
pub(super) async fn list(
    State(state): State<Arc<OrganizationState>>,
    Path(kind): Path<String>,
    Query(page): Query<Page>,
    headers: HeaderMap,
) -> Response {
    let corr = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, corr, None).await {
        Ok(p) => p,
        Err(s) => return error(s, corr),
    };
    let limit = page.limit.unwrap_or(100);
    if !kind_allowed(&kind) || !(1..=100).contains(&limit) {
        return error(StatusCode::BAD_REQUEST, corr);
    }
    let result: Result<Value, Error> = async {
        let ctx = proof.context();
        let (ResourceScope::Company(scope), Principal::User(actor)) = (ctx.scope, ctx.actor) else {
            return Err(Error::Forbidden);
        };
        let (t, c, a): (Uuid, Uuid, Uuid) =
            (scope.tenant.into(), scope.company.into(), actor.into());
        let mut tx = scoped_transaction(&state.runtime, ctx).await?;
        require_role(&mut tx, ctx, Role::OrgAdmin)
            .await
            .map_err(grant)?;
        recheck_authenticated(&mut tx, Some(&proof)).await?;
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT x::text FROM finance.staff_admin_list($1,$2,$3,$4,$5,$6) x")
                .bind(t)
                .bind(c)
                .bind(a)
                .bind(&kind)
                .bind(page.after.unwrap_or(Uuid::nil()))
                .bind(limit)
                .fetch_all(&mut *tx)
                .await
                .map_err(db_error)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|s| serde_json::from_str(s).map_err(|_| Error::Conflict))
            .collect::<Result<_, _>>()?;
        let next = if items.len() == limit as usize {
            items.last().and_then(|x| x.get("id")).cloned()
        } else {
            None
        };
        recheck_authenticated(&mut tx, Some(&proof)).await?;
        tx.commit().await?;
        Ok(json!({"items":items,"next_after":next}))
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(dossiers::status(e), corr),
    }
}
pub(super) async fn save(
    State(state): State<Arc<OrganizationState>>,
    Path(kind): Path<String>,
    headers: HeaderMap,
    body: Result<Json<Save>, JsonRejection>,
) -> Response {
    let corr = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, corr, None).await {
        Ok(p) => p,
        Err(s) => return error(s, corr),
    };
    let Ok(Json(input)) = body else {
        return error(StatusCode::BAD_REQUEST, corr);
    };
    let Some(key) = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| IdempotencyKey::parse(s).ok())
    else {
        return error(StatusCode::BAD_REQUEST, corr);
    };
    if !kind_allowed(&kind)
        || input.id.is_nil()
        || input
            .expected_version
            .is_some_and(|v| !(0..=9_007_199_254_740_990).contains(&v))
        || !payload_valid(&kind, input.expected_version, &input.payload)
    {
        return error(StatusCode::BAD_REQUEST, corr);
    }
    let result: Result<i64, Error> = async {
        let ctx = proof.context();
        let (ResourceScope::Company(scope), Principal::User(actor)) = (ctx.scope, ctx.actor) else {
            return Err(Error::Forbidden);
        };
        let (t, c, a): (Uuid, Uuid, Uuid) =
            (scope.tenant.into(), scope.company.into(), actor.into());
        let (cmd, correlation): (Uuid, Uuid) = (ctx.command.into(), ctx.correlation.into());
        let mut tx = scoped_transaction(&state.runtime, ctx).await?;
        require_role(&mut tx, ctx, Role::OrgAdmin)
            .await
            .map_err(grant)?;
        recheck_authenticated(&mut tx, Some(&proof)).await?;
        let version = sqlx::query_scalar(
            "SELECT finance.staff_admin_save($1,$2,$3,$4,$5,$6,$7,$8,$9,$10::text::jsonb)",
        )
        .bind(t)
        .bind(c)
        .bind(a)
        .bind(cmd)
        .bind(correlation)
        .bind(key.as_str())
        .bind(&kind)
        .bind(input.id)
        .bind(input.expected_version)
        .bind(input.payload.to_string())
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
        recheck_authenticated(&mut tx, Some(&proof)).await?;
        tx.commit().await?;
        Ok(version)
    }
    .await;
    match result {
        Ok(v) => Json(json!({"id":input.id,"version":v})).into_response(),
        Err(e) => error(dossiers::status(e), corr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_unrecognized_fields_or_legacy_role_injection() {
        assert!(payload_valid(
            "roles",
            None,
            &json!({"name":"Test","capabilities":["READ_OWN"]})
        ));
        assert!(!payload_valid(
            "roles",
            None,
            &json!({"name":"Test","roles":["ORG_ADMIN"]})
        ));
        assert!(!payload_valid(
            "assignments",
            Some(0),
            &json!({"active":true})
        ));
        assert!(!payload_valid(
            "invitations",
            None,
            &json!({"email":"x@example.invalid","roles":["ORG_ADMIN"]})
        ));
    }
}
