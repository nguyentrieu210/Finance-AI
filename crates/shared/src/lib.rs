#![forbid(unsafe_code)]
use serde::Serialize;

/// Operational liveness only. This does not assert business readiness.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Health {
    pub service: &'static str,
    pub status: &'static str,
    pub phase: &'static str,
}

pub const API_LIVENESS: Health = Health {
    service: "finance-api",
    status: "alive",
    phase: "foundation",
};

pub mod contracts;
pub mod ids;
pub mod money;

pub mod http_contract;

pub mod upload_contract;
