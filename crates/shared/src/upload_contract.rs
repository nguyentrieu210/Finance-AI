//! G01 technical draft: upload metadata and grant fencing. No storage access or authorization.
use crate::contracts::Version;
use chrono::{DateTime, Utc};

pub const DEFAULT_MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
pub const DEFAULT_MAX_DOSSIER_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sha256Digest([u8; 32]);
impl Sha256Digest {
    pub fn parse(value: &str) -> Result<Self, UploadError> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(UploadError::InvalidDigest);
        }
        let mut bytes = [0; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16)
                .map_err(|_| UploadError::InvalidDigest)?;
        }
        Ok(Self(bytes))
    }

    pub fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadError {
    InvalidDigest,
    InvalidSize,
    QuotaExceeded,
    UnsupportedMime,
    Closed,
    StaleGrant,
    GrantExpired,
    InvalidExpiry,
    VersionOverflow,
    ByteMismatch,
    DigestMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclaredMime {
    Pdf,
    Jpeg,
    Png,
    Docx,
    Xlsx,
}
impl DeclaredMime {
    pub fn parse(value: &str) -> Result<Self, UploadError> {
        match value {
            "application/pdf" => Ok(Self::Pdf),
            "image/jpeg" => Ok(Self::Jpeg),
            "image/png" => Ok(Self::Png),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
                Ok(Self::Docx)
            }
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Ok(Self::Xlsx),
            _ => Err(UploadError::UnsupportedMime),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadMetadata {
    bytes: u64,
    digest: Sha256Digest,
    mime: DeclaredMime,
}
impl UploadMetadata {
    pub fn new(
        bytes: u64,
        digest: Sha256Digest,
        mime: DeclaredMime,
        max_file_bytes: u64,
    ) -> Result<Self, UploadError> {
        if bytes == 0 || bytes > max_file_bytes {
            return Err(UploadError::InvalidSize);
        }
        Ok(Self {
            bytes,
            digest,
            mime,
        })
    }
    /// Caller must hold dossier quota lock; includes all outstanding intents, not just finalized files.
    pub fn check_quota(
        self,
        reserved_and_used: u64,
        max_dossier_bytes: u64,
    ) -> Result<(), UploadError> {
        match reserved_and_used.checked_add(self.bytes) {
            Some(sum) if sum <= max_dossier_bytes => Ok(()),
            _ => Err(UploadError::QuotaExceeded),
        }
    }
    /// Values must come from server verification of exact immutable staged object bytes.
    pub fn verify_bytes(
        self,
        actual_bytes: u64,
        actual_digest: Sha256Digest,
    ) -> Result<(), UploadError> {
        if actual_bytes != self.bytes {
            return Err(UploadError::ByteMismatch);
        }
        if actual_digest != self.digest {
            return Err(UploadError::DigestMismatch);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntentState {
    Pending,
    Finalized,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GrantFence {
    pub generation: Version,
    pub expires_at: DateTime<Utc>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadIntentGuard {
    pub state: IntentState,
    pub generation: Version,
}
impl UploadIntentGuard {
    /// A fresh grant invalidates all older generations. Each generation gets a separate staging key.
    /// The adapter must commit generation and expiry atomically; no URL or credentials live here.
    pub fn next_grant(
        self,
        now: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> Result<GrantFence, UploadError> {
        if self.state != IntentState::Pending {
            return Err(UploadError::Closed);
        }
        if expires_at <= now {
            return Err(UploadError::InvalidExpiry);
        }
        let next = i64::from(self.generation)
            .checked_add(1)
            .and_then(|n| Version::try_from(n).ok())
            .ok_or(UploadError::VersionOverflow)?;
        Ok(GrantFence {
            generation: next,
            expires_at,
        })
    }
    /// Must be called with the persisted current grant, not expiry or generation supplied by the client.
    /// The transaction rechecks the intent, dossier revision, scope, expiry and object version after verification.
    pub fn check_finalize(
        self,
        presented: Version,
        persisted: GrantFence,
        now: DateTime<Utc>,
    ) -> Result<(), UploadError> {
        if self.state != IntentState::Pending {
            return Err(UploadError::Closed);
        }
        if i64::from(self.generation) == 0
            || presented != self.generation
            || persisted.generation != self.generation
        {
            return Err(UploadError::StaleGrant);
        }
        if now >= persisted.expires_at {
            return Err(UploadError::GrantExpired);
        }
        Ok(())
    }
}
