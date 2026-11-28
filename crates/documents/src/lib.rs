#![forbid(unsafe_code)]
//! Isolated synthetic-only upload and quarantine prototype. No public API, DB,
//! object-store provider, parser, clean transition or download.
pub mod evidence;
pub mod repository;
pub mod scan;
use chrono::{DateTime, TimeDelta, Utc};
use finance_shared::{
    contracts::Version,
    ids::{CompanyScope, ResourceId, RevisionId},
    upload_contract::{
        DEFAULT_MAX_DOSSIER_BYTES, DEFAULT_MAX_FILE_BYTES, DeclaredMime, GrantFence, IntentState,
        Sha256Digest, UploadError, UploadIntentGuard, UploadMetadata,
    },
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub enum Error {
    Guard(UploadError),
    Scope,
    Revision,
    InvalidFilename,
    NoGrant,
    MimeMismatch,
    UnsupportedContainer,
    AlreadyExists,
    Storage(std::io::Error),
}
impl From<UploadError> for Error {
    fn from(e: UploadError) -> Self {
        Self::Guard(e)
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            Self::AlreadyExists
        } else {
            Self::Storage(e)
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Binding {
    pub scope: CompanyScope,
    pub document: ResourceId,
    pub revision: RevisionId,
}
/// Current state from a trusted aggregate, not a client assertion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionStatus {
    pub scope: CompanyScope,
    pub revision: RevisionId,
    pub editable: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Grant {
    binding: Binding,
    fence: GrantFence,
}
impl Grant {
    pub fn generation(self) -> Version {
        self.fence.generation
    }
    pub fn expires_at(self) -> DateTime<Utc> {
        self.fence.expires_at
    }
}
pub struct Intent {
    binding: Binding,
    metadata: UploadMetadata,
    mime: DeclaredMime,
    bytes: u64,
    guard: UploadIntentGuard,
    grant: Option<Grant>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Availability {
    Quarantined,
    Clean,
    Rejected,
}
/// Metadata only: intentionally no path, read API or clean verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Original {
    pub binding: Binding,
    pub bytes: u64,
    pub digest: Sha256Digest,
    pub mime: DeclaredMime,
    pub generation: Version,
    pub availability: Availability,
}
fn hash(bytes: &[u8]) -> Result<Sha256Digest, Error> {
    Ok(Sha256Digest::parse(&format!(
        "{:x}",
        Sha256::digest(bytes)
    ))?)
}
fn check_name(name: &str) -> Result<(), Error> {
    if name.trim() != name
        || name.is_empty()
        || name.len() > 255
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
    {
        return Err(Error::InvalidFilename);
    }
    Ok(())
}
/// A signature check, not format validation or malware scanning.
/// ZIP Office containers remain quarantined until the scanner/registry gates.
fn check_signature(mime: DeclaredMime, bytes: &[u8]) -> Result<(), Error> {
    let ok = match mime {
        DeclaredMime::Pdf => bytes.starts_with(b"%PDF-"),
        DeclaredMime::Jpeg => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        DeclaredMime::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        DeclaredMime::Docx | DeclaredMime::Xlsx => bytes.starts_with(b"PK\x03\x04"),
    };
    if !ok {
        return Err(Error::MimeMismatch);
    }
    Ok(())
}
impl Intent {
    /// Quota is a caller-supplied snapshot. No concurrent reservation guarantee in this isolated domain.
    pub fn create(
        binding: Binding,
        revision: RevisionStatus,
        filename: &str,
        bytes: u64,
        digest: Sha256Digest,
        mime: DeclaredMime,
        reserved_and_used: u64,
    ) -> Result<Self, Error> {
        check_name(filename)?;
        let metadata = UploadMetadata::new(bytes, digest, mime, DEFAULT_MAX_FILE_BYTES)?;
        metadata.check_quota(reserved_and_used, DEFAULT_MAX_DOSSIER_BYTES)?;
        let intent = Self {
            binding,
            metadata,
            mime,
            bytes,
            guard: UploadIntentGuard {
                state: IntentState::Pending,
                generation: Version::try_from(0).expect("valid zero"),
            },
            grant: None,
        };
        intent.check_revision(revision)?;
        Ok(intent)
    }
    pub fn state(&self) -> IntentState {
        self.guard.state
    }
    fn check_revision(&self, r: RevisionStatus) -> Result<(), Error> {
        if self.binding.scope != r.scope {
            return Err(Error::Scope);
        }
        if self.binding.revision != r.revision || !r.editable {
            return Err(Error::Revision);
        }
        Ok(())
    }
    pub fn issue_grant(
        &mut self,
        revision: RevisionStatus,
        now: DateTime<Utc>,
        ttl_seconds: i64,
    ) -> Result<Grant, Error> {
        self.check_revision(revision)?;
        if !(1..=300).contains(&ttl_seconds) {
            return Err(UploadError::InvalidExpiry.into());
        }
        let expires = now
            .checked_add_signed(TimeDelta::seconds(ttl_seconds))
            .ok_or(UploadError::InvalidExpiry)?;
        let fence = self.guard.next_grant(now, expires)?;
        let grant = Grant {
            binding: self.binding,
            fence,
        };
        self.guard.generation = fence.generation;
        self.grant = Some(grant);
        Ok(grant)
    }
    fn check_grant(
        &self,
        revision: RevisionStatus,
        grant: Grant,
        now: DateTime<Utc>,
    ) -> Result<(), Error> {
        self.check_revision(revision)?;
        if grant.binding != self.binding {
            return Err(Error::Scope);
        }
        let current = self.grant.ok_or(Error::NoGrant)?;
        self.guard
            .check_finalize(grant.generation(), current.fence, now)?;
        if grant != current {
            return Err(UploadError::StaleGrant.into());
        }
        Ok(())
    }
    fn verify_bytes(&self, bytes: &[u8]) -> Result<Sha256Digest, Error> {
        if bytes.len() as u64 != self.bytes || bytes.len() as u64 > DEFAULT_MAX_FILE_BYTES {
            return Err(UploadError::ByteMismatch.into());
        }
        let digest = hash(bytes)?;
        self.metadata.verify_bytes(bytes.len() as u64, digest)?;
        check_signature(self.mime, bytes)?;
        Ok(digest)
    }
    pub fn cancel(&mut self, revision: RevisionStatus) -> Result<(), Error> {
        self.check_revision(revision)?;
        if self.guard.state != IntentState::Pending {
            return Err(UploadError::Closed.into());
        }
        self.guard.state = IntentState::Cancelled;
        self.grant = None;
        Ok(())
    }
}
/// Local-filesystem proof of exclusive staging/promotion only. The root must be
/// an operator-created private disposable directory, with no untrusted writers.
/// Not an S3 adapter or a defense against a compromised filesystem administrator.
pub struct LocalQuarantine {
    root: PathBuf,
}
impl LocalQuarantine {
    pub fn open_private_root(root: &Path) -> Result<Self, Error> {
        let metadata = fs::symlink_metadata(root)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(Error::Scope);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(Error::Scope);
            }
        }
        Ok(Self {
            root: root.canonicalize()?,
        })
    }
    fn path(&self, b: Binding, g: Version, phase: &str) -> PathBuf {
        self.root.join(format!(
            "{}/{}/{}/{}/{}-{}",
            b.scope.tenant,
            b.scope.company,
            b.revision,
            b.document,
            phase,
            i64::from(g)
        ))
    }
    fn write_new(&self, path: &Path, bytes: &[u8]) -> Result<(), Error> {
        fs::create_dir_all(path.parent().ok_or(Error::Scope)?)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options.open(path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok(())
    }
    /// Bounded bytes already held by caller; no streaming HTTP gateway is exposed.
    pub fn stage(
        &self,
        intent: &Intent,
        revision: RevisionStatus,
        grant: Grant,
        now: DateTime<Utc>,
        bytes: &[u8],
    ) -> Result<(), Error> {
        intent.check_grant(revision, grant, now)?;
        intent.verify_bytes(bytes)?;
        self.write_new(
            &self.path(grant.binding, grant.generation(), "stage"),
            bytes,
        )
    }
    /// Re-read exact staged bytes, recompute length/hash/signature and write a NEW quarantine file.
    /// Mutable state is process-local. No atomic DB/object-store transaction or orphan recovery.
    pub fn finalize(
        &self,
        intent: &mut Intent,
        revision: RevisionStatus,
        grant: Grant,
        now: DateTime<Utc>,
    ) -> Result<Original, Error> {
        intent.check_grant(revision, grant, now)?;
        let f = File::open(self.path(grant.binding, grant.generation(), "stage"))?;
        let mut bytes = Vec::new();
        f.take(intent.bytes + 1).read_to_end(&mut bytes)?;
        let digest = intent.verify_bytes(&bytes)?;
        self.write_new(
            &self.path(grant.binding, grant.generation(), "quarantine"),
            &bytes,
        )?;
        intent.guard.state = IntentState::Finalized;
        Ok(Original {
            binding: intent.binding,
            bytes: intent.bytes,
            digest,
            mime: intent.mime,
            generation: grant.generation(),
            availability: Availability::Quarantined,
        })
    }
}
