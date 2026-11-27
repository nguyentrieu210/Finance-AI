//! Deployable pilot runtime. Configuration comes only from trusted process
//! environment and mounted files; client requests never select tenant/company.
use crate::organizations::{OrganizationState, router};
use axum::{Json, Router, http::StatusCode, routing::get};
use finance_identity::{Verifier, VerifierStore};
use finance_shared::{
    API_LIVENESS,
    ids::{CompanyId, CompanyScope, TenantId},
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Config,
    Database,
    Identity,
}

#[derive(Clone)]
pub struct Config {
    runtime_url: String,
    lookup_url: String,
    issuer: String,
    audience: String,
    jwks_file: PathBuf,
    jwks_ttl: u64,
    jwks_refresh: u64,
    scope: CompanyScope,
    cursor_key: [u8; 32],
}

fn now() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .map_err(|_| Error::Config)
}

fn bounded(value: String, max: usize) -> Result<String, Error> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        Err(Error::Config)
    } else {
        Ok(value)
    }
}

fn required(name: &'static str, max: usize) -> Result<String, Error> {
    bounded(env::var(name).map_err(|_| Error::Config)?, max)
}

fn uuid(name: &'static str) -> Result<Uuid, Error> {
    let value = Uuid::parse_str(&required(name, 64)?).map_err(|_| Error::Config)?;
    if value.is_nil() {
        Err(Error::Config)
    } else {
        Ok(value)
    }
}

fn number(name: &'static str, default: u64, min: u64, max: u64) -> Result<u64, Error> {
    let value = match env::var(name) {
        Ok(raw) => raw.parse::<u64>().map_err(|_| Error::Config)?,
        Err(_) => default,
    };
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(Error::Config)
    }
}

fn hex_key(raw: &str) -> Result<[u8; 32], Error> {
    if raw.len() != 64 {
        return Err(Error::Config);
    }
    let mut out = [0_u8; 32];
    for (index, pair) in raw.as_bytes().chunks_exact(2).enumerate() {
        let nibble = |value: u8| match value {
            b'0'..=b'9' => Some(value - b'0'),
            b'a'..=b'f' => Some(value - b'a' + 10),
            b'A'..=b'F' => Some(value - b'A' + 10),
            _ => None,
        };
        out[index] = nibble(pair[0])
            .and_then(|high| nibble(pair[1]).map(|low| (high << 4) | low))
            .ok_or(Error::Config)?;
    }
    Ok(out)
}

fn jwks(path: &Path) -> Result<String, Error> {
    if !path.is_absolute() {
        return Err(Error::Config);
    }
    let metadata = fs::metadata(path).map_err(|_| Error::Config)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 131_072 {
        return Err(Error::Config);
    }
    let value = fs::read_to_string(path).map_err(|_| Error::Config)?;
    if value.len() > 131_072 {
        return Err(Error::Config);
    }
    Ok(value)
}

impl Config {
    pub fn from_env() -> Result<Self, Error> {
        let runtime_url = required("FINANCE_RUNTIME_DATABASE_URL", 8192)?;
        let lookup_url = required("FINANCE_IDENTITY_DATABASE_URL", 8192)?;
        let issuer = required("FINANCE_OIDC_ISSUER", 512)?;
        let audience = required("FINANCE_OIDC_AUDIENCE", 256)?;
        let jwks_file = PathBuf::from(required("FINANCE_OIDC_JWKS_FILE", 4096)?);
        let jwks_ttl = number("FINANCE_OIDC_JWKS_TTL_SECONDS", 300, 30, 3600)?;
        let jwks_refresh = number("FINANCE_OIDC_JWKS_REFRESH_SECONDS", 60, 5, 900)?;
        if jwks_refresh >= jwks_ttl {
            return Err(Error::Config);
        }
        let tenant = TenantId::try_from(uuid("FINANCE_TENANT_ID")?).map_err(|_| Error::Config)?;
        let company =
            CompanyId::try_from(uuid("FINANCE_COMPANY_ID")?).map_err(|_| Error::Config)?;
        let cursor_key = hex_key(&required("FINANCE_CURSOR_KEY_HEX", 64)?)?;
        jwks(&jwks_file)?;
        Ok(Self {
            runtime_url,
            lookup_url,
            issuer,
            audience,
            jwks_file,
            jwks_ttl,
            jwks_refresh,
            scope: CompanyScope { tenant, company },
            cursor_key,
        })
    }
}

async fn connect(url: &str, role: &'static str) -> Result<PgPool, Error> {
    let role_statement = format!("SET ROLE {role}");
    PgPoolOptions::new()
        .max_connections(20)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .after_connect(move |connection, _| {
            let role_statement = role_statement.clone();
            Box::pin(async move {
                sqlx::query(&role_statement)
                    .execute(&mut *connection)
                    .await?;
                let allowed: bool = sqlx::query_scalar(
                    "SELECT current_user=$1 AND NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname IN (session_user,current_user) AND (rolsuper OR rolbypassrls OR rolcreaterole OR rolcreatedb))",
                )
                .bind(role)
                .fetch_one(&mut *connection)
                .await?;
                if !allowed {
                    return Err(sqlx::Error::Protocol("database role rejected".into()));
                }
                Ok(())
            })
        })
        .connect(url)
        .await
        .map_err(|_| Error::Database)
}

fn spawn_jwks_refresh(
    store: Arc<VerifierStore>,
    config: Config,
    valid_until: Arc<AtomicU64>,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(config.jwks_refresh));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let Ok(time) = now() else { continue };
            let Ok(raw) = jwks(&config.jwks_file) else {
                continue;
            };
            let Ok(next) = Verifier::from_trusted_snapshot(
                &config.issuer,
                &config.audience,
                &raw,
                time,
                config.jwks_ttl,
            ) else {
                continue;
            };
            if store.replace(next).is_ok() {
                valid_until.store(time.saturating_add(config.jwks_ttl), Ordering::Release);
            }
        }
    });
}

pub async fn build(config: Config) -> Result<Router, Error> {
    let time = now()?;
    let raw = jwks(&config.jwks_file)?;
    let verifier = Verifier::from_trusted_snapshot(
        &config.issuer,
        &config.audience,
        &raw,
        time,
        config.jwks_ttl,
    )
    .map_err(|_| Error::Identity)?;
    let store = Arc::new(VerifierStore::new(verifier));
    let runtime = connect(&config.runtime_url, "finance_runtime").await?;
    let lookup = connect(&config.lookup_url, "finance_identity_lookup").await?;
    let valid_until = Arc::new(AtomicU64::new(
        time.checked_add(config.jwks_ttl).ok_or(Error::Config)?,
    ));
    spawn_jwks_refresh(store.clone(), config.clone(), valid_until.clone());
    let readiness_runtime = runtime.clone();
    let readiness_lookup = lookup.clone();
    let state =
        OrganizationState::configured(runtime, lookup, store, config.scope, config.cursor_key);
    Ok(router(state)
        .route("/health/live", get(|| async { Json(API_LIVENESS) }))
        .route(
            "/health/ready",
            get(move || {
                let runtime = readiness_runtime.clone();
                let lookup = readiness_lookup.clone();
                let valid_until = valid_until.clone();
                async move {
                    let fresh =
                        now().is_ok_and(|value| value < valid_until.load(Ordering::Acquire));
                    let databases = tokio::time::timeout(Duration::from_secs(3), async {
                        sqlx::query("SELECT 1").execute(&runtime).await?;
                        sqlx::query("SELECT 1").execute(&lookup).await?;
                        Ok::<(), sqlx::Error>(())
                    })
                    .await
                    .is_ok_and(|value| value.is_ok());
                    if fresh && databases {
                        StatusCode::OK
                    } else {
                        StatusCode::SERVICE_UNAVAILABLE
                    }
                }
            }),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_secret_requires_exact_hex() {
        assert_eq!(hex_key(&"ab".repeat(32)).unwrap(), [0xab; 32]);
        assert!(hex_key(&"ab".repeat(31)).is_err());
        assert!(hex_key(&format!("{}zz", "ab".repeat(31))).is_err());
    }
}
