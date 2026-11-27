#![forbid(unsafe_code)]
//! Synthetic-only, offline trusted JWKS snapshot verifier and scoped identity resolver.
//! No discovery/network transport, login route, session issuance or permission grants.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use finance_shared::{
    contracts::{CommandContext, Principal, Version},
    ids::{ActorId, CommandId, CompanyScope, CorrelationId, ResourceScope},
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use sqlx::{PgPool, Postgres, Transaction};
use std::{
    collections::HashMap,
    sync::RwLock,
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    InvalidConfig,
    InvalidKeys,
    InvalidToken,
    ExpiredKeys,
    Forbidden,
    DatabaseUnavailable,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    alg: String,
    typ: String,
    kid: String,
}
#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}
#[derive(Deserialize)]
struct Jwk {
    kty: String,
    kid: String,
    alg: Option<String>,
    #[serde(rename = "use")]
    usage: Option<String>,
    key_ops: Option<Vec<String>>,
    n: Option<String>,
    e: Option<String>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}
impl Audience {
    fn contains(&self, value: &str) -> bool {
        match self {
            Self::One(v) => v == value,
            Self::Many(v) => !v.is_empty() && v.len() <= 16 && v.iter().any(|a| a == value),
        }
    }
}
#[derive(Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    aud: Audience,
    exp: u64,
    iat: u64,
    nbf: Option<u64>,
}
/// Trusted caller supplies fixed issuer/audience and snapshot fetched out of band.
pub struct Verifier {
    issuer: String,
    audience: String,
    keys: HashMap<String, DecodingKey>,
    loaded_at: u64,
    valid_until: u64,
}
/// Private, non-serializable proof of token validation. No raw token/claims exposed.
struct Verified {
    issuer: String,
    subject: String,
    expires: u64,
    keys_until: u64,
}

fn now() -> Result<u64, IdentityError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| IdentityError::InvalidConfig)
}
fn safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
}
fn bounded_text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
impl Verifier {
    /// Snapshot replacement is all-or-nothing. Removed keys no longer validate in the new snapshot.
    /// Restrict to RS256 signing keys and RFC 9068 at+jwt access tokens in this prototype.
    pub fn from_trusted_snapshot(
        issuer: &str,
        audience: &str,
        jwks: &str,
        loaded_at: u64,
        ttl_secs: u64,
    ) -> Result<Self, IdentityError> {
        if !issuer.starts_with("https://")
            || !bounded_text(issuer, 512)
            || issuer.contains(['?', '#', '@', ' '])
            || !bounded_text(audience, 256)
            || !(1..=3600).contains(&ttl_secs)
        {
            return Err(IdentityError::InvalidConfig);
        }
        if jwks.len() > 131072 {
            return Err(IdentityError::InvalidKeys);
        }
        let parsed: Jwks = serde_json::from_str(jwks).map_err(|_| IdentityError::InvalidKeys)?;
        if parsed.keys.is_empty() || parsed.keys.len() > 32 {
            return Err(IdentityError::InvalidKeys);
        }
        let mut keys = HashMap::new();
        for key in parsed.keys {
            if !safe_id(&key.kid)
                || key.kty != "RSA"
                || key.alg.as_deref().is_some_and(|v| v != "RS256")
                || key.usage.as_deref().is_some_and(|v| v != "sig")
                || key
                    .key_ops
                    .as_ref()
                    .is_some_and(|v| v.len() != 1 || v[0] != "verify")
            {
                return Err(IdentityError::InvalidKeys);
            }
            let n = key.n.ok_or(IdentityError::InvalidKeys)?;
            let e = key.e.ok_or(IdentityError::InvalidKeys)?;
            let nb = URL_SAFE_NO_PAD
                .decode(&n)
                .map_err(|_| IdentityError::InvalidKeys)?;
            let eb = URL_SAFE_NO_PAD
                .decode(&e)
                .map_err(|_| IdentityError::InvalidKeys)?;
            if !(256..=512).contains(&nb.len())
                || nb[0] & 0x80 == 0
                || nb.last().is_none_or(|v| v & 1 == 0)
                || eb.as_slice() != [1, 0, 1]
            {
                return Err(IdentityError::InvalidKeys);
            }
            let dk =
                DecodingKey::from_rsa_components(&n, &e).map_err(|_| IdentityError::InvalidKeys)?;
            if keys.insert(key.kid, dk).is_some() {
                return Err(IdentityError::InvalidKeys);
            }
        }
        Ok(Self {
            issuer: issuer.to_owned(),
            audience: audience.to_owned(),
            keys,
            loaded_at,
            valid_until: loaded_at
                .checked_add(ttl_secs)
                .ok_or(IdentityError::InvalidConfig)?,
        })
    }
    fn verify_at(&self, token: &str, now: u64) -> Result<Verified, IdentityError> {
        if now < self.loaded_at || now >= self.valid_until {
            return Err(IdentityError::ExpiredKeys);
        }
        if token.len() > 16384 {
            return Err(IdentityError::InvalidToken);
        }
        let mut parts = token.split('.');
        let header = parts.next().ok_or(IdentityError::InvalidToken)?;
        let payload = parts.next().ok_or(IdentityError::InvalidToken)?;
        let sig = parts.next().ok_or(IdentityError::InvalidToken)?;
        if parts.next().is_some()
            || header.is_empty()
            || payload.is_empty()
            || sig.is_empty()
            || header.len() > 2048
        {
            return Err(IdentityError::InvalidToken);
        }
        let header: Header = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(header)
                .map_err(|_| IdentityError::InvalidToken)?,
        )
        .map_err(|_| IdentityError::InvalidToken)?;
        if header.alg != "RS256" || header.typ != "at+jwt" || !safe_id(&header.kid) {
            return Err(IdentityError::InvalidToken);
        }
        let key = self
            .keys
            .get(&header.kid)
            .ok_or(IdentityError::InvalidToken)?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.issuer]);
        validation.set_audience(&[&self.audience]);
        validation.set_required_spec_claims(&["exp", "iss", "sub", "aud"]);
        // Use injected trusted clock below, not the library wall clock. Signature verification stays enabled.
        validation.validate_exp = false;
        validation.validate_nbf = false;
        validation.leeway = 0;
        let data =
            decode::<Claims>(token, key, &validation).map_err(|_| IdentityError::InvalidToken)?;
        let c = data.claims;
        if c.iss != self.issuer
            || !c.aud.contains(&self.audience)
            || !bounded_text(&c.sub, 256)
            || c.exp <= now
            || c.iat > now
            || c.exp <= c.iat
            || c.exp - c.iat > 3600
            || c.nbf.is_some_and(|nbf| nbf > now || nbf >= c.exp)
        {
            return Err(IdentityError::InvalidToken);
        }
        Ok(Verified {
            issuer: c.iss,
            subject: c.sub,
            expires: c.exp,
            keys_until: self.valid_until,
        })
    }
    /// Validate signature and then resolve existing active membership. Scope is a selection,
    /// not a claim granting access. Identity lookup never auto-provisions users or companies.
    pub async fn authenticate(
        &self,
        pool: &PgPool,
        token: &str,
        scope: CompanyScope,
    ) -> Result<Principal, IdentityError> {
        let verified = self.verify_at(token, now()?)?;
        resolve(pool, &verified, scope).await
    }
}
/// Atomic trusted snapshot replacement, with fixed issuer/audience and monotonically newer load time.
/// A bad refresh never extends the old snapshot's expiry. Requests already verified before rotation
/// may complete; this is not session revocation or a live IdP refresh service.
pub struct VerifierStore {
    current: RwLock<Verifier>,
}
impl VerifierStore {
    pub fn new(initial: Verifier) -> Self {
        Self {
            current: RwLock::new(initial),
        }
    }
    pub fn replace(&self, next: Verifier) -> Result<(), IdentityError> {
        let mut current = self
            .current
            .write()
            .map_err(|_| IdentityError::InvalidConfig)?;
        if next.issuer != current.issuer
            || next.audience != current.audience
            || next.loaded_at <= current.loaded_at
        {
            return Err(IdentityError::InvalidConfig);
        }
        *current = next;
        Ok(())
    }
    fn verify_at(&self, token: &str, now: u64) -> Result<Verified, IdentityError> {
        self.current
            .read()
            .map_err(|_| IdentityError::InvalidConfig)?
            .verify_at(token, now)
    }
    pub async fn authenticate(
        &self,
        pool: &PgPool,
        token: &str,
        scope: CompanyScope,
    ) -> Result<Principal, IdentityError> {
        let verified = self.verify_at(token, now()?)?;
        resolve(pool, &verified, scope).await
    }
}
/// Opaque request capability; never deserialize it or accept actor/scope from a request body.
/// Issuer/subject are kept private. No raw bearer token is retained.
pub struct AuthenticatedCommand {
    context: CommandContext,
    issuer: String,
    subject: String,
    valid_until: u64,
}
impl AuthenticatedCommand {
    pub fn context(&self) -> &CommandContext {
        &self.context
    }
    /// Call after current-grant locks and immediately before transaction commit/replay.
    /// Existing identity row is held by the permission guard; this check is not a grant by itself.
    pub async fn recheck(&self, tx: &mut Transaction<'_, Postgres>) -> Result<(), IdentityError> {
        let ResourceScope::Company(scope) = self.context.scope else {
            return Err(IdentityError::Forbidden);
        };
        let Principal::User(actor) = self.context.actor else {
            return Err(IdentityError::Forbidden);
        };
        let (t, c, a): (Uuid, Uuid, Uuid) =
            (scope.tenant.into(), scope.company.into(), actor.into());
        let deadline = i64::try_from(self.valid_until).map_err(|_| IdentityError::InvalidToken)?;
        if now()? >= self.valid_until {
            return Err(IdentityError::InvalidToken);
        }
        let allowed:bool=sqlx::query_scalar("SELECT COALESCE(current_user='finance_runtime' AND current_setting('transaction_isolation')='read committed' AND finance.scope_tenant()=$1 AND finance.scope_company()=$2 AND finance.scope_actor()=$3 AND extract(epoch FROM clock_timestamp())<$6::bigint AND EXISTS(SELECT 1 FROM finance.identity_subject WHERE tenant_id=$1 AND id=$3 AND active AND issuer=$4 AND subject=$5),false)")
            .bind(t).bind(c).bind(a).bind(&self.issuer).bind(&self.subject).bind(deadline)
            .fetch_one(&mut **tx).await.map_err(|_|IdentityError::DatabaseUnavailable)?;
        if !allowed {
            return Err(IdentityError::Forbidden);
        }
        Ok(())
    }
}
impl VerifierStore {
    /// Trusted server supplies command and correlation IDs; actor comes only from validated JWT + DB.
    pub async fn authenticate_command(
        &self,
        pool: &PgPool,
        token: &str,
        scope: CompanyScope,
        command: CommandId,
        correlation: CorrelationId,
        expected_version: Option<Version>,
    ) -> Result<AuthenticatedCommand, IdentityError> {
        let verified = self.verify_at(token, now()?)?;
        let actor = resolve(pool, &verified, scope).await?;
        let n = now()?;
        let deadline = verified.expires.min(verified.keys_until);
        if n >= deadline {
            return Err(IdentityError::InvalidToken);
        }
        let occurred_at = chrono::DateTime::from_timestamp(
            i64::try_from(n).map_err(|_| IdentityError::InvalidConfig)?,
            0,
        )
        .ok_or(IdentityError::InvalidConfig)?;
        Ok(AuthenticatedCommand {
            context: CommandContext {
                command,
                correlation,
                actor,
                scope: ResourceScope::Company(scope),
                expected_version,
                occurred_at,
            },
            issuer: verified.issuer,
            subject: verified.subject,
            valid_until: deadline,
        })
    }
}
async fn resolve(
    pool: &PgPool,
    identity: &Verified,
    scope: CompanyScope,
) -> Result<Principal, IdentityError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|_| IdentityError::DatabaseUnavailable)?;
    let role:bool=sqlx::query_scalar("SELECT current_user='finance_identity_lookup' AND NOT rolsuper AND NOT rolbypassrls AND NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname=session_user AND (rolsuper OR rolbypassrls OR rolcreaterole)) FROM pg_roles WHERE rolname=current_user").fetch_one(&mut *tx).await.map_err(|_|IdentityError::DatabaseUnavailable)?;
    if !role {
        return Err(IdentityError::Forbidden);
    }
    let (t, c): (Uuid, Uuid) = (scope.tenant.into(), scope.company.into());
    sqlx::query("SELECT set_config('finance.tenant_id',$1,true),set_config('finance.company_id',$2,true),set_config('finance.identity_issuer',$3,true),set_config('finance.identity_subject',$4,true)")
        .bind(t.to_string()).bind(c.to_string()).bind(&identity.issuer).bind(&identity.subject).execute(&mut *tx).await.map_err(|_|IdentityError::DatabaseUnavailable)?;
    let expires = i64::try_from(identity.expires).map_err(|_| IdentityError::InvalidToken)?;
    let actor:Option<Uuid>=sqlx::query_scalar("SELECT i.id FROM finance.identity_subject i JOIN finance.tenant t ON t.id=i.tenant_id JOIN finance.company c ON c.tenant_id=i.tenant_id AND c.id=$2 WHERE i.tenant_id=$1 AND i.issuer=$3 AND i.subject=$4 AND i.active AND t.active AND c.active AND extract(epoch FROM statement_timestamp())<$5::bigint AND EXISTS(SELECT 1 FROM finance.membership m WHERE m.tenant_id=$1 AND m.company_id=$2 AND m.actor_id=i.id AND m.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))")
        .bind(t).bind(c).bind(&identity.issuer).bind(&identity.subject).bind(expires).fetch_optional(&mut *tx).await.map_err(|_|IdentityError::DatabaseUnavailable)?;
    let actor = ActorId::try_from(actor.ok_or(IdentityError::Forbidden)?)
        .map_err(|_| IdentityError::Forbidden)?;
    tx.commit()
        .await
        .map_err(|_| IdentityError::DatabaseUnavailable)?;
    Ok(Principal::User(actor))
}

#[cfg(test)]
mod tests;
