//! Explicitly opt-in signed bridge protocol, not a claimed vendor API.
//! DO submits only a sealed digest; ERP is read-only. No payment endpoint.
use crate::ports::{
    self, DoCapabilities, DoDispatch, DoReceipt, DoStatus, ErpEvent, ErpEventKind, ErpSnapshot,
};
use finance_shared::{
    ids::{CompanyScope, ResourceId},
    money::Money,
    upload_contract::Sha256Digest,
};
use hmac::{Hmac, Mac};
use reqwest::{Client, Method, Url};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::Duration;
use uuid::Uuid;
type Hmac256 = Hmac<Sha256>;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Disabled,
    Configuration,
    Transport,
    Uncertain,
    Contract,
    Signature,
}
pub struct Bridge {
    base: Url,
    client: Client,
    key: Vec<u8>,
    scope: CompanyScope,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Signed {
    payload: String,
    signature: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capabilities {
    external_idempotency: bool,
    signed_webhook: bool,
    authenticated_polling: bool,
}
#[derive(Serialize)]
struct Dispatch {
    tenant: Uuid,
    company: Uuid,
    handoff: Uuid,
    revision: Uuid,
    command: Uuid,
    digest: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    tenant: Uuid,
    company: Uuid,
    command: Uuid,
    external_id: Uuid,
    status: String,
    source_digest: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    tenant: Uuid,
    company: Uuid,
    snapshot_id: Uuid,
    source_digest: String,
    as_of_unix_s: u64,
    fresh_until_unix_s: u64,
    complete: bool,
    events: Vec<Event>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    external_id: Uuid,
    sequence: u64,
    kind: String,
    amount: Money,
    source_digest: String,
    reverses: Option<Uuid>,
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn decode(s: &str) -> Result<Vec<u8>, Error> {
    if s.len() != 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Signature);
    }
    (0..32)
        .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| Error::Signature))
        .collect()
}
impl Bridge {
    /// Caller supplies explicit approval and a tenant-scoped secret. The custom
    /// bridge must be implemented/approved by the actual DO/ERP integration owner.
    pub fn new(
        approved: bool,
        endpoint: &str,
        key: &[u8],
        scope: CompanyScope,
    ) -> Result<Self, Error> {
        if !approved {
            return Err(Error::Disabled);
        }
        let base = Url::parse(endpoint).map_err(|_| Error::Configuration)?;
        if base.scheme() != "https"
            || base.host_str().is_none()
            || base.path() != "/"
            || base.query().is_some()
            || base.fragment().is_some()
            || !base.username().is_empty()
            || base.password().is_some()
            || !(32..=128).contains(&key.len())
        {
            return Err(Error::Configuration);
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| Error::Configuration)?;
        Ok(Self {
            base,
            client,
            key: key.to_vec(),
            scope,
        })
    }
    fn sign(&self, domain: &str, request_id: Uuid, payload: &str) -> Result<Hmac256, Error> {
        let mut mac = Hmac256::new_from_slice(&self.key).map_err(|_| Error::Configuration)?;
        mac.update(domain.as_bytes());
        mac.update(&[0]);
        mac.update(request_id.as_bytes());
        mac.update(Uuid::from(self.scope.tenant).as_bytes());
        mac.update(Uuid::from(self.scope.company).as_bytes());
        mac.update(payload.as_bytes());
        Ok(mac)
    }
    async fn exchange<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        id: Uuid,
        body: String,
    ) -> Result<T, Error> {
        if id.is_nil() || body.len() > 262144 {
            return Err(Error::Contract);
        }
        let url = self.base.join(path).map_err(|_| Error::Configuration)?;
        if url.origin() != self.base.origin() {
            return Err(Error::Configuration);
        }
        let signature = hex(&self
            .sign(
                &format!("finance.bridge.request.v1/{method}/{path}"),
                id,
                &body,
            )?
            .finalize()
            .into_bytes());
        let mut response = self
            .client
            .request(method, url)
            .header("x-finance-request-id", id.to_string())
            .header("x-finance-tenant", self.scope.tenant.to_string())
            .header("x-finance-company", self.scope.company.to_string())
            .header("x-finance-signature", signature)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| Error::Transport)?;
        if !response.status().is_success() {
            return Err(Error::Uncertain);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Transport)? {
            if chunk.len() > 8 * 1024 * 1024 - bytes.len() {
                return Err(Error::Contract);
            }
            bytes.extend_from_slice(&chunk);
        }
        let signed: Signed = serde_json::from_slice(&bytes).map_err(|_| Error::Contract)?;
        self.sign(
            &format!("finance.bridge.response.v1/{path}"),
            id,
            &signed.payload,
        )?
        .verify_slice(&decode(&signed.signature)?)
        .map_err(|_| Error::Signature)?;
        serde_json::from_str(&signed.payload).map_err(|_| Error::Contract)
    }
    pub async fn capabilities(&self, id: Uuid) -> Result<DoCapabilities, Error> {
        let r: Capabilities = self
            .exchange(Method::GET, "/v1/do/capabilities", id, String::new())
            .await?;
        let result = DoCapabilities {
            external_idempotency: r.external_idempotency,
            signed_webhook: r.signed_webhook,
            authenticated_polling: r.authenticated_polling,
        };
        ports::validate_do_capabilities(result).map_err(|_| Error::Contract)?;
        if !result.authenticated_polling {
            return Err(Error::Contract);
        }
        Ok(result)
    }
    pub async fn dispatch(
        &self,
        command: DoDispatch,
        capability_id: Uuid,
    ) -> Result<DoReceipt, Error> {
        if command.scope != self.scope {
            return Err(Error::Contract);
        }
        self.capabilities(capability_id).await?;
        let id: Uuid = command.command_id.into();
        let body = serde_json::to_string(&Dispatch {
            tenant: self.scope.tenant.into(),
            company: self.scope.company.into(),
            handoff: command.handoff.into(),
            revision: command.revision.into(),
            command: id,
            digest: hex(&command.payload_digest.as_bytes()),
        })
        .map_err(|_| Error::Contract)?;
        let reply: Receipt = self
            .exchange(Method::POST, "/v1/do/dispatch", id, body)
            .await
            .map_err(|_| Error::Uncertain)?;
        self.receipt(reply, id)
    }
    pub async fn lookup(&self, command: ResourceId, request_id: Uuid) -> Result<DoReceipt, Error> {
        let reply: Receipt = self
            .exchange(
                Method::GET,
                &format!("/v1/do/status/{command}"),
                request_id,
                String::new(),
            )
            .await?;
        self.receipt(reply, command.into())
    }
    fn receipt(&self, r: Receipt, command: Uuid) -> Result<DoReceipt, Error> {
        if r.tenant != Uuid::from(self.scope.tenant)
            || r.company != Uuid::from(self.scope.company)
            || r.command != command
        {
            return Err(Error::Contract);
        }
        let status = match r.status.as_str() {
            "PENDING" => DoStatus::Pending,
            "UNKNOWN" => DoStatus::Unknown,
            "ACKNOWLEDGED" => DoStatus::Acknowledged,
            "APPROVED" => DoStatus::Approved,
            "REJECTED" => DoStatus::Rejected,
            _ => return Err(Error::Contract),
        };
        Ok(DoReceipt {
            external_id: ResourceId::try_from(r.external_id).map_err(|_| Error::Contract)?,
            status,
            source_digest: Sha256Digest::parse(&r.source_digest).map_err(|_| Error::Contract)?,
            authoritative: true,
        })
    }
    pub async fn erp_snapshot(&self, id: Uuid, now: u64) -> Result<ErpSnapshot, Error> {
        let r: Snapshot = self
            .exchange(Method::GET, "/v1/erp/snapshot", id, String::new())
            .await?;
        if r.tenant != Uuid::from(self.scope.tenant)
            || r.company != Uuid::from(self.scope.company)
            || r.events.len() > 100000
        {
            return Err(Error::Contract);
        }
        let mut events = Vec::with_capacity(r.events.len());
        for e in r.events {
            events.push(ErpEvent {
                external_id: ResourceId::try_from(e.external_id).map_err(|_| Error::Contract)?,
                sequence: e.sequence,
                kind: match e.kind.as_str() {
                    "PAYMENT" => ErpEventKind::Payment,
                    "SETTLEMENT" => ErpEventKind::Settlement,
                    "REVERSAL" => ErpEventKind::Reversal,
                    _ => return Err(Error::Contract),
                },
                amount: e.amount,
                source_digest: Sha256Digest::parse(&e.source_digest)
                    .map_err(|_| Error::Contract)?,
                reverses: e
                    .reverses
                    .map(ResourceId::try_from)
                    .transpose()
                    .map_err(|_| Error::Contract)?,
            });
        }
        let mut snapshot = ErpSnapshot {
            scope: self.scope,
            snapshot_id: ResourceId::try_from(r.snapshot_id).map_err(|_| Error::Contract)?,
            source_digest: Sha256Digest::parse(&r.source_digest).map_err(|_| Error::Contract)?,
            as_of_unix_s: r.as_of_unix_s,
            fresh_until_unix_s: r.fresh_until_unix_s,
            complete: r.complete,
            events,
        };
        snapshot.events = ports::validate_erp_snapshot(self.scope, now, &snapshot)
            .map_err(|_| Error::Contract)?;
        Ok(snapshot)
    }
}
