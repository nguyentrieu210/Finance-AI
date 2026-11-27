//! Explicit loopback-only disposable fixture runtime. Never a production mode.
use crate::organizations::{OrganizationState, router};
use axum::{Json, Router, http::StatusCode, routing::get};
use finance_identity::{Verifier, VerifierStore};
use finance_shared::{API_LIVENESS, ids::{CompanyId, CompanyScope, TenantId}};
use rand::{RngCore, rngs::OsRng};
use sqlx::{PgPool, postgres::{PgConnectOptions, PgPoolOptions, PgSslMode}};
use std::{net::SocketAddr, time::{Duration, SystemTime, UNIX_EPOCH}};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Config,
    Database,
    Identity,
}

fn now() -> Result<u64, Error> {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d|d.as_secs()).map_err(|_|Error::Config)
}

/// Closed URL grammar for the disposable fixture in test_prototype_foundation.sh.
/// No arbitrary DSN/query options/remote host/user/database accepted.
fn options(raw: &str, login: &str) -> Result<PgConnectOptions, Error> {
    let prefix=format!("postgresql://{login}:");
    let (password, tail)=raw.strip_prefix(&prefix).and_then(|s|s.split_once("@127.0.0.1:")).ok_or(Error::Config)?;
    let port=tail.strip_suffix("/finance_prototype?sslmode=disable").and_then(|s|s.parse::<u16>().ok()).filter(|n|*n>0).ok_or(Error::Config)?;
    if password.len()!=48 || !password.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)){
        return Err(Error::Config);
    }
    Ok(PgConnectOptions::new()
        .host("127.0.0.1")
        .port(port)
        .database("finance_prototype")
        .username(login)
        .password(password)
        .ssl_mode(PgSslMode::Disable))
}

pub fn bind_address(raw: &str) -> Result<SocketAddr, Error> {
    let address:SocketAddr=raw.parse().map_err(|_|Error::Config)?;
    if !matches!(address.ip(),std::net::IpAddr::V4(ip) if ip.is_loopback()) {
        return Err(Error::Config);
    }
    Ok(address)
}

async fn connect(raw: &str, login: &'static str, role: &'static str, statement: &'static str) -> Result<PgPool, Error> {
    let config=options(raw,login)?;
    PgPoolOptions::new()
        .max_connections(4)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .after_connect(move |conn,_|Box::pin(async move{
            sqlx::query(statement).execute(&mut *conn).await?;
            let allowed:bool=sqlx::query_scalar("SELECT current_database()='finance_prototype' AND session_user=$1 AND current_user=$2 AND NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname IN (session_user,current_user) AND (rolsuper OR rolbypassrls OR rolcreaterole OR rolcreatedb))")
                .bind(login).bind(role).fetch_one(&mut *conn).await?;
            if !allowed{
                return Err(sqlx::Error::Protocol("synthetic database role rejected".into()));
            }
            Ok(())
        }))
        .connect_with(config).await.map_err(|_|Error::Database)
}

/// Public keys only, fixed synthetic issuer and tenant/company fixture IDs.
/// A successful readiness response means this sandbox is reachable, not
/// that any business/production release gate has been approved.
pub async fn build(runtime_url: &str, lookup_url: &str, jwks: &str) -> Result<Router, Error> {
    let time=now()?;
    let expires=time.checked_add(600).ok_or(Error::Config)?;
    let verifier=VerifierStore::new(Verifier::from_trusted_snapshot("https://synthetic.invalid","finance-api",jwks,time,600).map_err(|_|Error::Identity)?);
    let runtime=connect(runtime_url,"finance_test_login","finance_runtime","SET ROLE finance_runtime").await?;
    let lookup=connect(lookup_url,"finance_identity_test_login","finance_identity_lookup","SET ROLE finance_identity_lookup").await?;
    let scope=CompanyScope{
        tenant:TenantId::try_from(Uuid::from_u128(1)).map_err(|_|Error::Config)?,
        company:CompanyId::try_from(Uuid::from_u128(10)).map_err(|_|Error::Config)?,
    };
    let mut cursor=[0u8;32];
    OsRng.fill_bytes(&mut cursor);
    let readiness_runtime=runtime.clone();
    let readiness_lookup=lookup.clone();
    let app=router(OrganizationState::synthetic(runtime,lookup,verifier,scope,cursor));
    Ok(app
        .route("/health/live",get(||async{Json(API_LIVENESS)}))
        .route("/health/ready",get(move || {
            let runtime=readiness_runtime.clone();
            let lookup=readiness_lookup.clone();
            async move {
                if now().is_ok_and(|n|n<expires)
                    && tokio::time::timeout(Duration::from_secs(3),async {
                        sqlx::query("SELECT 1").execute(&runtime).await?;
                        sqlx::query("SELECT 1").execute(&lookup).await?;
                        Ok::<(),sqlx::Error>(())
                    }).await.is_ok_and(|r|r.is_ok())
                {
                    StatusCode::OK
                }else{
                    StatusCode::SERVICE_UNAVAILABLE
                }
            }
        })))
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn rejects_arbitrary_runtime_targets(){
        let pw="a".repeat(48);
        let valid=format!("postgresql://finance_test_login:{pw}@127.0.0.1:5432/finance_prototype?sslmode=disable");
        assert!(options(&valid,"finance_test_login").is_ok());
        for bad in [
            valid.replace("127.0.0.1","192.0.2.1"),
            valid.replace("finance_prototype","production"),
            valid.replace("finance_test_login","postgres"),
            valid.replace("?sslmode=disable","?sslmode=disable&host=elsewhere"),
            valid.replace(":5432/",":0/"),
        ] {
            assert!(options(&bad,"finance_test_login").is_err());
        }
        assert!(bind_address("127.0.0.1:0").is_ok());
        assert!(bind_address("0.0.0.0:8080").is_err());
        assert!(bind_address("[::]:8080").is_err());
    }
}
