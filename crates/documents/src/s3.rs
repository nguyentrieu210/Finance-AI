//! Version-pinned S3-compatible storage adapter. Configuration is supplied
//! by an authorized service, never by an upload client. No public ACLs,
//! redirects, mutable-alias reads, deletion, or real-provider activation.
use super::{Binding, check_signature};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use finance_shared::{
    contracts::Version,
    upload_contract::{DEFAULT_MAX_FILE_BYTES, DeclaredMime, Sha256Digest},
};
use hmac::{Hmac, Mac};
use reqwest::{
    Client, Method, Url,
    header::{HeaderMap, HeaderValue},
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, time::Duration};

type Hmac256 = Hmac<Sha256>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Configuration,
    Metadata,
    InvalidVersion,
    Transport,
    Provider,
    Conflict,
    Integrity,
}

pub struct Object {
    pub binding: Binding,
    pub generation: Version,
    pub size: u64,
    pub mime: DeclaredMime,
    pub digest: Sha256Digest,
}

#[derive(Clone, Copy)]
pub enum Phase {
    Staging,
    Quarantine,
}

pub struct Grant {
    pub url: Url,
    pub headers: HeaderMap,
}

#[derive(Clone)]
pub struct S3 {
    endpoint: Url,
    region: String,
    access: String,
    secret: String,
    session: Option<String>,
    staging: String,
    quarantine: String,
    client: Client,
}

fn valid_token(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            use std::fmt::Write;
            write!(out, "%{b:02X}").expect("write to string");
        }
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(output, "{b:02x}").expect("write to string");
    }
    output
}

fn mac(key: &[u8], message: &str) -> Vec<u8> {
    let mut mac = Hmac256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn mime(m: DeclaredMime) -> &'static str {
    match m {
        DeclaredMime::Pdf => "application/pdf",
        DeclaredMime::Jpeg => "image/jpeg",
        DeclaredMime::Png => "image/png",
        DeclaredMime::Docx => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        }
        DeclaredMime::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    }
}

fn version(v: &str) -> Result<&str, Error> {
    if v.is_empty() || v == "null" || v.len() > 1024 || !v.bytes().all(|b| (33..=126).contains(&b))
    {
        return Err(Error::InvalidVersion);
    }
    Ok(v)
}

impl Object {
    fn key(&self, phase: Phase) -> Result<String, Error> {
        let generation = i64::from(self.generation);
        if generation < 1 || !(1..=DEFAULT_MAX_FILE_BYTES).contains(&self.size) {
            return Err(Error::Metadata);
        }
        let phase = match phase {
            Phase::Staging => "staging",
            Phase::Quarantine => "quarantine",
        };
        Ok(format!(
            "{phase}/{}/{}/{}/{}/{generation}",
            self.binding.scope.tenant,
            self.binding.scope.company,
            self.binding.revision,
            self.binding.document
        ))
    }

    fn verify(&self, content: &[u8]) -> Result<(), Error> {
        if content.len() as u64 != self.size
            || Sha256::digest(content).as_slice() != self.digest.as_bytes()
            || check_signature(self.mime, content).is_err()
        {
            return Err(Error::Integrity);
        }
        Ok(())
    }
}

impl S3 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        endpoint: &str,
        region: &str,
        access: &str,
        secret: &str,
        session: Option<&str>,
        staging: &str,
        quarantine: &str,
    ) -> Result<Self, Error> {
        let url = Url::parse(endpoint).map_err(|_| Error::Configuration)?;
        let host = url.host_str().ok_or(Error::Configuration)?;
        let loopback = host == "127.0.0.1";
        if host.contains(':') {
            return Err(Error::Configuration);
        }
        let bucket = |b: &str| {
            (3..=63).contains(&b.len())
                && b.bytes()
                    .all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == b'-')
                && !b.starts_with('-')
                && !b.ends_with('-')
        };
        if !(url.scheme() == "https" || (loopback && url.scheme() == "http"))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || !valid_token(region, 64)
            || !valid_token(access, 128)
            || secret.is_empty()
            || secret.len() > 256
            || secret.chars().any(char::is_control)
            || session
                .is_some_and(|s| s.is_empty() || s.len() > 4096 || s.chars().any(char::is_control))
            || !bucket(staging)
            || !bucket(quarantine)
            || staging == quarantine
        {
            return Err(Error::Configuration);
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|_| Error::Configuration)?;
        Ok(Self {
            endpoint: url,
            region: region.into(),
            access: access.into(),
            secret: secret.into(),
            session: session.map(str::to_owned),
            staging: staging.into(),
            quarantine: quarantine.into(),
            client,
        })
    }

    fn presign(
        &self,
        method: Method,
        object: &Object,
        phase: Phase,
        version_id: Option<&str>,
        now: DateTime<Utc>,
        ttl: u32,
    ) -> Result<Grant, Error> {
        if !(1..=300).contains(&ttl) {
            return Err(Error::Metadata);
        }
        let bucket = match phase {
            Phase::Staging => &self.staging,
            Phase::Quarantine => &self.quarantine,
        };
        let path = format!("/{bucket}/{}", object.key(phase)?);
        let host = self.endpoint.host_str().ok_or(Error::Configuration)?;
        let authority = match self.endpoint.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_owned(),
        };
        let mut headers = BTreeMap::from([("host", authority)]);
        if method == Method::PUT {
            if version_id.is_some() {
                return Err(Error::Metadata);
            }
            headers.insert("content-length", object.size.to_string());
            headers.insert("content-type", mime(object.mime).into());
            headers.insert("if-none-match", "*".into());
            headers.insert(
                "x-amz-checksum-sha256",
                STANDARD.encode(object.digest.as_bytes()),
            );
        } else if !((method == Method::GET && version_id.is_some())
            || (method == Method::HEAD && version_id.is_none()))
        {
            return Err(Error::Metadata);
        }
        let signed = headers.keys().copied().collect::<Vec<_>>().join(";");
        use std::fmt::Write;
        let mut canonical_headers = String::new();
        for (k, v) in &headers {
            writeln!(canonical_headers, "{k}:{v}").expect("write to string");
        }
        let day = now.format("%Y%m%d").to_string();
        let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
        let scope = format!("{day}/{}/s3/aws4_request", self.region);
        let mut pairs = vec![
            ("X-Amz-Algorithm", "AWS4-HMAC-SHA256".into()),
            ("X-Amz-Credential", format!("{}/{scope}", self.access)),
            ("X-Amz-Date", stamp.clone()),
            ("X-Amz-Expires", ttl.to_string()),
            ("X-Amz-SignedHeaders", signed.clone()),
        ];
        if let Some(token) = &self.session {
            pairs.push(("X-Amz-Security-Token", token.clone()));
        }
        if let Some(v) = version_id {
            pairs.push(("versionId", version(v)?.to_owned()));
        }
        let mut pairs: Vec<_> = pairs
            .into_iter()
            .map(|(k, v)| (encode(k), encode(&v)))
            .collect();
        pairs.sort();
        let query = pairs
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        let canonical =
            format!("{method}\n{path}\n{query}\n{canonical_headers}\n{signed}\nUNSIGNED-PAYLOAD");
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{stamp}\n{scope}\n{}",
            hex(&Sha256::digest(canonical))
        );
        let date_key = mac(format!("AWS4{}", self.secret).as_bytes(), &day);
        let region_key = mac(&date_key, &self.region);
        let service_key = mac(&region_key, "s3");
        let key = mac(&service_key, "aws4_request");
        let signature = hex(&mac(&key, &string_to_sign));
        let url = Url::parse(&format!(
            "{}://{}{}?{}&X-Amz-Signature={signature}",
            self.endpoint.scheme(),
            headers["host"],
            path,
            query
        ))
        .map_err(|_| Error::Configuration)?;
        let mut result = HeaderMap::new();
        for (name, value) in headers {
            if name != "host" {
                result.insert(
                    reqwest::header::HeaderName::from_bytes(name.as_bytes())
                        .map_err(|_| Error::Configuration)?,
                    HeaderValue::from_str(&value).map_err(|_| Error::Configuration)?,
                );
            }
        }
        Ok(Grant {
            url,
            headers: result,
        })
    }

    /// Explicit operator opt-in. Missing or partial configuration fails closed.
    pub fn from_env() -> Result<Option<Self>, Error> {
        match std::env::var("FINANCE_S3_ENABLED") {
            Err(std::env::VarError::NotPresent) => return Ok(None),
            Ok(value) if value == "0" => return Ok(None),
            Ok(value) if value == "1" => {}
            _ => return Err(Error::Configuration),
        }
        let get = |name| std::env::var(name).map_err(|_| Error::Configuration);
        let token = std::env::var("FINANCE_S3_SESSION_TOKEN").ok();
        Self::new(
            &get("FINANCE_S3_ENDPOINT")?,
            &get("FINANCE_S3_REGION")?,
            &get("FINANCE_S3_ACCESS_KEY")?,
            &get("FINANCE_S3_SECRET_KEY")?,
            token.as_deref(),
            &get("FINANCE_S3_STAGING_BUCKET")?,
            &get("FINANCE_S3_QUARANTINE_BUCKET")?,
        )
        .map(Some)
    }

    /// Discover an immutable version at a conditional-write key. HEAD alone
    /// never proves the content: all callers must re-read the returned version.
    async fn current_version(
        &self,
        object: &Object,
        phase: Phase,
        now: DateTime<Utc>,
    ) -> Result<String, Error> {
        let grant = self.presign(Method::HEAD, object, phase, None, now, 60)?;
        let response = self
            .client
            .head(grant.url)
            .headers(grant.headers)
            .send()
            .await
            .map_err(|_| Error::Transport)?;
        if !response.status().is_success() {
            return Err(Error::Provider);
        }
        if response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            != Some(object.size)
            || response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                != Some(mime(object.mime))
        {
            return Err(Error::Integrity);
        }
        Ok(version(
            response
                .headers()
                .get("x-amz-version-id")
                .and_then(|v| v.to_str().ok())
                .ok_or(Error::InvalidVersion)?,
        )?
        .to_owned())
    }

    pub async fn finalize_staged(
        &self,
        object: &Object,
        now: DateTime<Utc>,
    ) -> Result<(String, String), Error> {
        let staged = self.current_version(object, Phase::Staging, now).await?;
        match self.promote(object, &staged, now).await {
            Ok(original) => Ok((staged, original)),
            Err(Error::Conflict) => {
                // Recovery after a lost receipt/DB rollback: accept only the
                // exact bytes under the same scoped immutable generation.
                let original = self.current_version(object, Phase::Quarantine, now).await?;
                self.read_verified(object, Phase::Quarantine, &original, now)
                    .await?;
                Ok((staged, original))
            }
            Err(error) => Err(error),
        }
    }

    /// Upload exactly once; client must send all signed headers and the exact bytes.
    pub fn upload_grant(
        &self,
        object: &Object,
        now: DateTime<Utc>,
        ttl: u32,
    ) -> Result<Grant, Error> {
        self.presign(Method::PUT, object, Phase::Staging, None, now, ttl)
    }

    pub async fn read_verified(
        &self,
        object: &Object,
        phase: Phase,
        version_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<u8>, Error> {
        let grant = self.presign(Method::GET, object, phase, Some(version_id), now, 60)?;
        let mut response = self
            .client
            .get(grant.url)
            .headers(grant.headers)
            .send()
            .await
            .map_err(|_| Error::Transport)?;
        if !response.status().is_success() {
            return Err(Error::Provider);
        }
        if response
            .headers()
            .get("x-amz-version-id")
            .and_then(|v| v.to_str().ok())
            != Some(version_id)
            || response.content_length() != Some(object.size)
            || response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                != Some(mime(object.mime))
        {
            return Err(Error::Integrity);
        }
        let mut bytes = Vec::with_capacity(object.size as usize);
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Transport)? {
            if chunk.len() > object.size as usize - bytes.len() {
                return Err(Error::Integrity);
            }
            bytes.extend_from_slice(&chunk);
        }
        object.verify(&bytes)?;
        Ok(bytes)
    }

    /// Promote the exact verified staging version, using a conditional new
    /// object. Re-read the exact returned quarantine version before returning.
    pub async fn promote(
        &self,
        object: &Object,
        staged_version: &str,
        now: DateTime<Utc>,
    ) -> Result<String, Error> {
        let bytes = self
            .read_verified(object, Phase::Staging, staged_version, now)
            .await?;
        let grant = self.presign(Method::PUT, object, Phase::Quarantine, None, now, 300)?;
        let response = self
            .client
            .put(grant.url)
            .headers(grant.headers)
            .body(bytes)
            .send()
            .await
            .map_err(|_| Error::Transport)?;
        if matches!(response.status().as_u16(), 409 | 412) {
            return Err(Error::Conflict);
        }
        if !response.status().is_success() {
            return Err(Error::Provider);
        }
        let id = version(
            response
                .headers()
                .get("x-amz-version-id")
                .and_then(|v| v.to_str().ok())
                .ok_or(Error::InvalidVersion)?,
        )?
        .to_owned();
        self.read_verified(object, Phase::Quarantine, &id, now)
            .await?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_shared::ids::{CompanyId, CompanyScope, ResourceId, RevisionId, TenantId};
    use uuid::Uuid;

    fn object() -> Object {
        let bytes = b"%PDF-1.7 test";
        Object {
            binding: Binding {
                scope: CompanyScope {
                    tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
                    company: CompanyId::try_from(Uuid::from_u128(2)).unwrap(),
                },
                revision: RevisionId::try_from(Uuid::from_u128(3)).unwrap(),
                document: ResourceId::try_from(Uuid::from_u128(4)).unwrap(),
            },
            generation: Version::try_from(1).unwrap(),
            size: bytes.len() as u64,
            mime: DeclaredMime::Pdf,
            digest: Sha256Digest::parse(&hex(&Sha256::digest(bytes))).unwrap(),
        }
    }

    #[test]
    fn grants_sign_all_constraints_and_reads_pin_version() {
        let s3 = S3::new(
            "http://127.0.0.1:9000",
            "us-east-1",
            "test-key",
            "test-secret",
            None,
            "stage",
            "quarantine",
        )
        .unwrap();
        let now = DateTime::parse_from_rfc3339("2026-11-28T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let object = object();
        let grant = s3.upload_grant(&object, now, 60).unwrap();
        let query: BTreeMap<_, _> = grant.url.query_pairs().into_owned().collect();
        assert_eq!(
            query["X-Amz-SignedHeaders"],
            "content-length;content-type;host;if-none-match;x-amz-checksum-sha256"
        );
        assert_eq!(grant.headers["if-none-match"], "*");
        assert_eq!(query["X-Amz-Signature"].len(), 64);
        let read = s3
            .presign(
                Method::GET,
                &object,
                Phase::Staging,
                Some("version+/="),
                now,
                60,
            )
            .unwrap();
        assert!(
            read.url
                .query_pairs()
                .any(|(k, v)| k == "versionId" && v == "version+/=")
        );
        assert!(
            s3.presign(Method::GET, &object, Phase::Staging, Some("null"), now, 60)
                .is_err()
        );
        assert!(s3.upload_grant(&object, now, 301).is_err());
        assert!(object.verify(b"%PDF-1.7 test").is_ok());
        assert_eq!(object.verify(b"%PDF-1.7 fake"), Err(Error::Integrity));
        assert!(
            S3::new(
                "http://example.com",
                "us-east-1",
                "test-key",
                "test-secret",
                None,
                "stage",
                "quarantine"
            )
            .is_err()
        );
    }
}
