//! Opt-in local synthetic AI transport, bounded and isolated from workflow decisions.
use crate::gateway::{Approval, Request, authorize};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, time::Duration};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error { Unauthorized, Configuration, Input, Transport, Provider, Output }

#[derive(Serialize)]
struct Envelope<'a> {
    schema: &'static str,
    model: &'a str,
    prompt: &'a str,
    input: &'a str,
    max_output_tokens: u32,
    tools_enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub field: String,
    pub value: String,
    pub evidence_quote: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply { schema: String, model: String, candidates: Vec<Candidate> }

pub struct Gateway {
    endpoint: Url,
    client: Client,
}

impl Gateway {
    /// The provider contract is an isolated loopback service. Remote provider
    /// selection and real-document processing require owner approval.
    pub fn new(endpoint: &str) -> Result<Self, Error> {
        let endpoint = Url::parse(endpoint).map_err(|_| Error::Configuration)?;
        if endpoint.scheme() != "http" || endpoint.host_str() != Some("127.0.0.1")
            || endpoint.port().is_none() || endpoint.path() != "/v1/extract"
            || endpoint.query().is_some() || endpoint.fragment().is_some()
            || !endpoint.username().is_empty() || endpoint.password().is_some()
        { return Err(Error::Configuration); }
        let client = Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3)).build().map_err(|_| Error::Configuration)?;
        Ok(Self { endpoint, client })
    }

    pub async fn extract(&self, approval: Option<&Approval<'_>>, request: &Request<'_>,
        prompt: &str, minimized_input: &str, allowed_fields: &BTreeSet<String>) -> Result<Vec<Candidate>, Error> {
        authorize(approval, request).map_err(|_| Error::Unauthorized)?;
        if request.input_bytes != minimized_input.len() || prompt.is_empty() || prompt.len() > 65536
            || Sha256::digest(prompt.as_bytes()).as_slice() != request.prompt.as_bytes()
            || allowed_fields.is_empty() || allowed_fields.len() > 128
        { return Err(Error::Input); }
        let payload = Envelope { schema: "finance.ai.extract.v1", model: request.model,
            prompt, input: minimized_input, max_output_tokens: request.output_tokens, tools_enabled: false };
        let mut response = self.client.post(self.endpoint.clone()).timeout(Duration::from_secs(u64::from(request.timeout_seconds)))
            .json(&payload).send().await.map_err(|_| Error::Transport)?;
        if !response.status().is_success() { return Err(Error::Provider); }
        let limit = usize::try_from(request.output_tokens).map_err(|_| Error::Input)? * 32 + 4096;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Transport)? {
            if chunk.len() > limit - bytes.len() { return Err(Error::Output); }
            bytes.extend_from_slice(&chunk);
        }
        validate(&bytes, request.model, minimized_input, allowed_fields)
    }
}

fn validate(bytes: &[u8], model: &str, input: &str, allowed: &BTreeSet<String>) -> Result<Vec<Candidate>, Error> {
    let response: Reply = serde_json::from_slice(bytes).map_err(|_| Error::Output)?;
    if response.schema != "finance.ai.candidates.v1" || response.model != model || response.candidates.len() > allowed.len() {
        return Err(Error::Output);
    }
    let mut seen = BTreeSet::new();
    for candidate in &response.candidates {
        if !allowed.contains(&candidate.field) || !seen.insert(&candidate.field)
            || candidate.value.trim().is_empty() || candidate.value.len() > 512
            || candidate.evidence_quote.trim().is_empty() || candidate.evidence_quote.len() > 4096
            || !input.contains(&candidate.evidence_quote)
            || candidate.value.chars().any(char::is_control)
        { return Err(Error::Output); }
    }
    // Candidate values remain unverified, even when evidence quotation is real.
    Ok(response.candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_fabricated_evidence_unknown_fields_and_duplicate_values() {
        let fields = BTreeSet::from(["invoice.total".to_owned()]);
        let good = br#"{"schema":"finance.ai.candidates.v1","model":"fixture","candidates":[{"field":"invoice.total","value":"100","evidence_quote":"Total: 100"}]}"#;
        assert!(validate(good,"fixture","Total: 100",&fields).is_ok());
        assert!(validate(good,"fixture","Total: 900",&fields).is_err());
        assert!(validate(good,"different","Total: 100",&fields).is_err());
        assert!(Gateway::new("http://example.com/v1/extract").is_err());
        let unknown = br#"{"schema":"finance.ai.candidates.v1","model":"fixture","candidates":[{"field":"approved","value":"true","evidence_quote":"Total: 100"}]}"#;
        assert!(validate(unknown,"fixture","Total: 100",&fields).is_err());
    }
}
