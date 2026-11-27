//! Explicitly injected synthetic organization read router. Not mounted by the default API.
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use finance_core::prototype::{
    CreateUnit, Error as DomainError, UnitKind, create_unit_authenticated,
};
use finance_identity::{IdentityError, VerifierStore};
use finance_organization::{get_unit_authenticated, list_units_authenticated};
use finance_shared::{
    contracts::{Principal, Version},
    http_contract::{IdempotencyKey, VersionTarget, VersionToken},
    ids::{ActorId, CommandId, CompanyScope, CorrelationId, ResourceId},
};
use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::PgPool;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

mod audit;
mod checks;
mod dashboard;
mod documents;
mod dossiers;
mod exports;
mod fields;
mod forms;
mod integration;
mod jobs;
mod knowledge;
mod lifecycle;
mod matches;
mod memberships;
mod policies;
mod references;
mod reservations;
mod reviews;
mod risks;
mod rules;
mod templates;

type Hmac256 = Hmac<Sha256>;

pub struct OrganizationState {
    runtime: PgPool,
    lookup: PgPool,
    verifier: Arc<VerifierStore>,
    scope: CompanyScope,
    cursor_key: [u8; 32],
}

impl OrganizationState {
    /// Scope is trusted server configuration, never selected by a client header.
    pub fn synthetic(
        runtime: PgPool,
        lookup: PgPool,
        verifier: VerifierStore,
        scope: CompanyScope,
        cursor_key: [u8; 32],
    ) -> Self {
        Self::configured(runtime, lookup, Arc::new(verifier), scope, cursor_key)
    }

    pub(crate) fn configured(
        runtime: PgPool,
        lookup: PgPool,
        verifier: Arc<VerifierStore>,
        scope: CompanyScope,
        cursor_key: [u8; 32],
    ) -> Self {
        Self {
            runtime,
            lookup,
            verifier,
            scope,
            cursor_key,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    limit: Option<i64>,
    cursor: Option<String>,
}

#[derive(Serialize)]
struct Organization {
    id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<Uuid>,
    kind: String,
    code: String,
    name: String,
    active: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum CreateOrganizationKind {
    Company,
    Department,
    CostCenter,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateOrganization {
    kind: CreateOrganizationKind,
    code: String,
    name: String,
    parent_id: Option<Uuid>,
}

#[derive(Serialize)]
struct Page {
    items: Vec<Organization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

#[derive(Serialize)]
struct Failure {
    code: &'static str,
    message: &'static str,
    correlation_id: Uuid,
}

fn error(status: StatusCode, correlation_id: Uuid) -> Response {
    let (code, message) = match status {
        StatusCode::UNAUTHORIZED => ("UNAUTHENTICATED", "Authentication required"),
        StatusCode::FORBIDDEN => ("FORBIDDEN", "Access denied"),
        StatusCode::BAD_REQUEST => ("INVALID_INPUT", "Invalid input"),
        StatusCode::CONFLICT => ("CONFLICT", "Command conflict"),
        StatusCode::UNPROCESSABLE_ENTITY => ("BUSINESS_GUARD", "Business guard failed"),
        StatusCode::NOT_FOUND => ("NOT_FOUND", "Resource not found"),
        _ => ("UNAVAILABLE", "Dependency unavailable"),
    };
    (
        status,
        Json(Failure {
            code,
            message,
            correlation_id,
        }),
    )
        .into_response()
}

fn random_id() -> Uuid {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn now() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn sign(key: &[u8; 32], scope: CompanyScope, actor: ActorId, payload: &[u8]) -> Hmac256 {
    let mut mac = Hmac256::new_from_slice(key).expect("fixed HMAC key");
    mac.update(b"finance.org.cursor/v1");
    for id in [
        Uuid::from(scope.tenant),
        Uuid::from(scope.company),
        Uuid::from(actor),
    ] {
        mac.update(id.as_bytes());
    }
    mac.update(payload);
    mac
}

fn encode_cursor(
    key: &[u8; 32],
    scope: CompanyScope,
    actor: ActorId,
    after: Uuid,
    expires: u64,
) -> String {
    let mut bytes = Vec::with_capacity(56);
    bytes.extend_from_slice(after.as_bytes());
    bytes.extend_from_slice(&expires.to_be_bytes());
    bytes.extend_from_slice(&sign(key, scope, actor, &bytes).finalize().into_bytes());
    URL_SAFE_NO_PAD.encode(bytes)
}

fn decode_cursor(
    key: &[u8; 32],
    scope: CompanyScope,
    actor: ActorId,
    value: &str,
    now: u64,
) -> Option<Uuid> {
    if value.len() > 100 {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(value).ok()?;
    if bytes.len() != 56 {
        return None;
    }
    sign(key, scope, actor, &bytes[..24])
        .verify_slice(&bytes[24..])
        .ok()?;
    let expires = u64::from_be_bytes(bytes[16..24].try_into().ok()?);
    if expires <= now || expires > now.checked_add(300)? {
        return None;
    }
    Some(Uuid::from_bytes(bytes[..16].try_into().ok()?))
}

async fn list(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    query: Result<Query<PageQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let correlation = random_id();
    let token = match headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
    {
        Some(t) if !t.is_empty() && t.len() <= 16_384 => t,
        _ => return error(StatusCode::UNAUTHORIZED, correlation),
    };
    let proof = match state
        .verifier
        .authenticate_command(
            &state.lookup,
            token,
            state.scope,
            CommandId::try_from(random_id()).expect("random command"),
            CorrelationId::try_from(correlation).expect("random correlation"),
            None,
        )
        .await
    {
        Ok(p) => p,
        Err(IdentityError::InvalidToken) => return error(StatusCode::UNAUTHORIZED, correlation),
        Err(IdentityError::Forbidden) => return error(StatusCode::FORBIDDEN, correlation),
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, correlation),
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
        None => Uuid::nil(),
        Some(value) => match decode_cursor(&state.cursor_key, state.scope, actor, &value, time) {
            Some(id) => id,
            None => return error(StatusCode::BAD_REQUEST, correlation),
        },
    };
    let rows = match list_units_authenticated(&state.runtime, &proof, after, limit).await {
        Ok(rows) => rows,
        Err(DomainError::Forbidden) => return error(StatusCode::FORBIDDEN, correlation),
        Err(DomainError::InvalidInput) => return error(StatusCode::BAD_REQUEST, correlation),
        Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, correlation),
    };
    // A full final page may yield one extra empty page. No unscoped count query.
    let next_cursor = if rows.len() == limit as usize {
        rows.last()
            .map(|u| encode_cursor(&state.cursor_key, state.scope, actor, u.id, time + 300))
    } else {
        None
    };
    let items = rows
        .into_iter()
        .map(|u| Organization {
            id: u.id,
            parent_id: u.parent,
            kind: u.kind,
            code: u.code,
            name: u.name,
            active: u.active,
        })
        .collect();
    let mut response = Json(Page { items, next_cursor }).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

fn stable_organization_id(
    scope: CompanyScope,
    actor: ActorId,
    key: &IdempotencyKey,
) -> Uuid {
    let mut digest = Sha256::new();
    digest.update(b"finance.organization.http-id/v1");
    digest.update(Uuid::from(scope.tenant).as_bytes());
    digest.update(Uuid::from(scope.company).as_bytes());
    digest.update(Uuid::from(actor).as_bytes());
    digest.update(key.as_str().as_bytes());
    let hash = digest.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

async fn create_organization(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    body: Result<Json<CreateOrganization>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    let Principal::User(actor) = proof.context().actor else {
        return error(StatusCode::FORBIDDEN, correlation);
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
    let kind = match body.kind {
        CreateOrganizationKind::Company => {
            return error(StatusCode::UNPROCESSABLE_ENTITY, correlation);
        }
        CreateOrganizationKind::Department => UnitKind::Department,
        CreateOrganizationKind::CostCenter => UnitKind::CostCenter,
    };
    if body.parent_id.is_some_and(|id| id.is_nil()) {
        return error(StatusCode::BAD_REQUEST, correlation);
    }
    let id = stable_organization_id(state.scope, actor, &key);
    let kind_text = match kind {
        UnitKind::Department => "DEPARTMENT",
        UnitKind::CostCenter => "COST_CENTER",
    };
    let input = CreateUnit {
        id,
        parent: body.parent_id,
        code: &body.code,
        name: &body.name,
        kind,
    };
    match create_unit_authenticated(&state.runtime, &proof, &key, &input).await {
        Ok(_) => {
            let version = Version::try_from(0).expect("zero version");
            let token = VersionToken {
                target: VersionTarget::Organization,
                resource_id: ResourceId::try_from(id).expect("derived nonnil organization"),
                version,
            };
            let mut response = (
                StatusCode::CREATED,
                Json(Organization {
                    id,
                    parent_id: body.parent_id,
                    kind: kind_text.to_owned(),
                    code: body.code,
                    name: body.name,
                    active: true,
                }),
            )
                .into_response();
            response.headers_mut().insert(
                "finance-version",
                axum::http::HeaderValue::from_str(&token.to_string())
                    .expect("canonical organization version"),
            );
            response.headers_mut().insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            );
            response
        }
        Err(DomainError::Forbidden) => error(StatusCode::FORBIDDEN, correlation),
        Err(DomainError::InvalidInput) => error(StatusCode::BAD_REQUEST, correlation),
        Err(DomainError::Conflict) => error(StatusCode::CONFLICT, correlation),
        Err(DomainError::Database(_)) => error(StatusCode::SERVICE_UNAVAILABLE, correlation),
    }
}

async fn get_organization(
    State(state): State<Arc<OrganizationState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Response {
    let correlation = random_id();
    let proof = match dossiers::authenticate_version(&state, &headers, correlation, None).await {
        Ok(proof) => proof,
        Err(status) => return error(status, correlation),
    };
    match get_unit_authenticated(&state.runtime, &proof, id).await {
        Ok(Some(unit)) => {
            let Ok(version) = finance_shared::contracts::Version::try_from(unit.version) else {
                return error(StatusCode::SERVICE_UNAVAILABLE, correlation);
            };
            let token = finance_shared::http_contract::VersionToken {
                target: finance_shared::http_contract::VersionTarget::Organization,
                resource_id: finance_shared::ids::ResourceId::try_from(unit.id)
                    .expect("stored nonnil organization"),
                version,
            };
            let mut response = Json(Organization {
                id: unit.id,
                parent_id: unit.parent,
                kind: unit.kind,
                code: unit.code,
                name: unit.name,
                active: unit.active,
            })
            .into_response();
            response.headers_mut().insert(
                "finance-version",
                axum::http::HeaderValue::from_str(&token.to_string())
                    .expect("canonical organization version"),
            );
            response.headers_mut().insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            );
            response
        }
        Ok(None) => error(StatusCode::NOT_FOUND, correlation),
        Err(DomainError::Forbidden) => error(StatusCode::FORBIDDEN, correlation),
        Err(DomainError::InvalidInput) => error(StatusCode::BAD_REQUEST, correlation),
        Err(_) => error(StatusCode::SERVICE_UNAVAILABLE, correlation),
    }
}

pub fn router(state: OrganizationState) -> Router {
    Router::new()
        .route("/v1/organizations", get(list).post(create_organization))
        .route("/v1/organizations/{item_id}", get(get_organization))
        .route(
            "/v1/memberships",
            get(memberships::list).post(memberships::create),
        )
        .route("/v1/memberships/{item_id}", get(memberships::get))
        .route(
            "/v1/memberships/{item_id}/revoke",
            axum::routing::post(memberships::revoke),
        )
        .route("/v1/dossiers", get(dossiers::list).post(dossiers::create))
        .route("/v1/dossiers/{dossier_id}", get(dossiers::get))
        .route(
            "/v1/dossiers/{dossier_id}/documents",
            get(documents::list).post(documents::create),
        )
        .route("/v1/dossiers/{dossier_id}/checks", get(checks::list))
        .route("/v1/dossiers/{dossier_id}/fields", get(fields::list))
        .route(
            "/v1/dossiers/{dossier_id}/matches",
            get(matches::list).post(matches::confirm),
        )
        .route(
            "/v1/dossiers/{dossier_id}/reservations",
            get(reservations::list),
        )
        .route(
            "/v1/dossiers/{dossier_id}/fields/decisions",
            axum::routing::post(fields::decide),
        )
        .route(
            "/v1/dossiers/{dossier_id}/ready",
            axum::routing::post(lifecycle::ready),
        )
        .route(
            "/v1/dossiers/{dossier_id}/snapshot",
            get(lifecycle::snapshot),
        )
        .route(
            "/v1/dossiers/{dossier_id}/exports",
            get(exports::list).post(exports::create),
        )
        .route(
            "/v1/dossiers/{dossier_id}/begin-review",
            axum::routing::post(reviews::begin),
        )
        .route(
            "/v1/dossiers/{dossier_id}/review",
            get(reviews::get).post(reviews::decide),
        )
        .route("/v1/review/queue", get(reviews::queue))
        .route("/v1/jobs/{job_id}", get(jobs::get))
        .route("/v1/audit", get(audit::list))
        .route("/v1/dashboard", get(dashboard::get))
        .route("/__diagnostics/knowledge", get(knowledge::search))
        .route("/v1/forms", get(forms::list).post(forms::create))
        .route("/v1/forms/{item_id}", get(forms::get))
        .route(
            "/v1/forms/{item_id}/revise",
            axum::routing::post(forms::revise),
        )
        .route(
            "/v1/templates",
            get(templates::list).post(templates::create),
        )
        .route("/v1/templates/{item_id}", get(templates::get))
        .route(
            "/v1/templates/{item_id}/revise",
            axum::routing::post(templates::revise),
        )
        .route("/v1/policies", get(policies::list))
        .route("/v1/policies/{item_id}", get(policies::get))
        .route("/v1/contracts", get(references::contracts))
        .route("/v1/advances", get(references::advances))
        .route("/v1/budgets", get(references::budgets))
        .route("/v1/rules", get(rules::list).post(rules::create))
        .route("/v1/rules/{item_id}", get(rules::get))
        .route("/v1/risks", get(risks::list_risks))
        .route("/v1/risks/{item_id}", get(risks::get_risk))
        .route(
            "/v1/dossiers/{dossier_id}/waivers",
            get(risks::list_waivers).post(risks::request_waiver),
        )
        .route(
            "/v1/dossiers/{dossier_id}/waivers/{waiver_id}/decision",
            axum::routing::post(risks::decide_waiver),
        )
        .route(
            "/v1/dossiers/{dossier_id}/handoffs",
            get(integration::list_handoffs).post(integration::create_handoff),
        )
        .route("/v1/reconciliation", get(integration::list_reconciliation))
        .route(
            "/v1/reconciliation/{item_id}",
            get(integration::get_reconciliation),
        )
        .route(
            "/v1/reconciliation/{item_id}/resolve",
            axum::routing::post(integration::resolve_reconciliation),
        )
        .layer(axum::extract::DefaultBodyLimit::max(8192))
        .with_state(Arc::new(state))
}
