use chrono::{DateTime, Utc};
use finance_documents::*;
use finance_shared::{ids::*, upload_contract::*};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use uuid::Uuid;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "finance-quarantine-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut b = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            b.mode(0o700);
        }
        b.create(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn binding(c: u128, d: u128, r: u128) -> Binding {
    Binding {
        scope: CompanyScope {
            tenant: TenantId::try_from(id(1)).unwrap(),
            company: CompanyId::try_from(id(c)).unwrap(),
        },
        document: ResourceId::try_from(id(d)).unwrap(),
        revision: RevisionId::try_from(id(r)).unwrap(),
    }
}
fn revision(b: Binding) -> RevisionStatus {
    RevisionStatus {
        scope: b.scope,
        revision: b.revision,
        editable: true,
    }
}
fn at(t: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(t, 0).unwrap()
}
fn digest(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::parse(&format!("{:x}", Sha256::digest(bytes))).unwrap()
}
const PDF: &[u8] = b"%PDF-1.7\nSynthetic signature only; never parsed.\n";
fn create(b: Binding, bytes: &[u8]) -> Intent {
    Intent::create(
        b,
        revision(b),
        "synthetic.pdf",
        bytes.len() as u64,
        digest(bytes),
        DeclaredMime::Pdf,
        0,
    )
    .unwrap()
}
fn path(root: &Temp, b: Binding, g: i64, phase: &str) -> PathBuf {
    root.0.join(format!(
        "{}/{}/{}/{}/{}-{}",
        b.scope.tenant, b.scope.company, b.revision, b.document, phase, g
    ))
}
#[test]
fn real_bytes_promoted_once_and_never_replaced() {
    let root = Temp::new();
    let store = LocalQuarantine::open_private_root(&root.0).unwrap();
    let b = binding(10, 20, 30);
    let mut i = create(b, PDF);
    let g = i.issue_grant(revision(b), at(100), 300).unwrap();
    store.stage(&i, revision(b), g, at(101), PDF).unwrap();
    assert!(matches!(
        store.stage(&i, revision(b), g, at(102), PDF),
        Err(Error::AlreadyExists)
    ));
    let original = store.finalize(&mut i, revision(b), g, at(103)).unwrap();
    assert_eq!(original.availability, Availability::Quarantined);
    assert_eq!(original.digest, digest(PDF));
    assert_eq!(fs::read(path(&root, b, 1, "quarantine")).unwrap(), PDF);
    assert_eq!(i.state(), IntentState::Finalized);
    assert!(i.issue_grant(revision(b), at(104), 100).is_err());
    assert!(store.finalize(&mut i, revision(b), g, at(104)).is_err());
    let mut replacement = create(binding(10, 21, 31), b"%PDF-1.7\nreplacement");
    let r = revision(binding(10, 21, 31));
    let g2 = replacement.issue_grant(r, at(105), 60).unwrap();
    store
        .stage(&replacement, r, g2, at(106), b"%PDF-1.7\nreplacement")
        .unwrap();
    store.finalize(&mut replacement, r, g2, at(107)).unwrap();
    assert_eq!(fs::read(path(&root, b, 1, "quarantine")).unwrap(), PDF);
}
#[test]
fn stale_expired_cross_scope_and_sealed_revisions_denied() {
    let root = Temp::new();
    let store = LocalQuarantine::open_private_root(&root.0).unwrap();
    let b = binding(10, 20, 30);
    let r = revision(b);
    let mut i = create(b, PDF);
    let g1 = i.issue_grant(r, at(100), 60).unwrap();
    store.stage(&i, r, g1, at(101), PDF).unwrap();
    let g2 = i.issue_grant(r, at(102), 60).unwrap();
    assert!(matches!(
        store.finalize(&mut i, r, g1, at(103)),
        Err(Error::Guard(UploadError::StaleGrant))
    ));
    assert!(matches!(
        store.stage(&i, r, g2, at(162), PDF),
        Err(Error::Guard(UploadError::GrantExpired))
    ));
    assert!(matches!(
        store.stage(&i, revision(binding(11, 20, 30)), g2, at(103), PDF),
        Err(Error::Scope)
    ));
    assert!(matches!(
        store.stage(
            &i,
            RevisionStatus {
                editable: false,
                ..r
            },
            g2,
            at(103),
            PDF
        ),
        Err(Error::Revision)
    ));
    assert!(matches!(
        store.stage(&i, revision(binding(10, 20, 31)), g2, at(103), PDF),
        Err(Error::Revision)
    ));
    i.cancel(r).unwrap();
    assert!(store.stage(&i, r, g2, at(103), PDF).is_err());
    assert!(!path(&root, b, 1, "quarantine").exists());
}
#[test]
fn wrong_checksum_length_signature_and_staging_tampering_fail() {
    let root = Temp::new();
    let store = LocalQuarantine::open_private_root(&root.0).unwrap();
    let b = binding(10, 20, 30);
    let r = revision(b);
    let mut i = create(b, PDF);
    let g = i.issue_grant(r, at(100), 60).unwrap();
    assert!(matches!(
        store.stage(&i, r, g, at(101), b"bad"),
        Err(Error::Guard(UploadError::ByteMismatch))
    ));
    let mut wrong = PDF.to_vec();
    wrong[10] ^= 1;
    assert!(matches!(
        store.stage(&i, r, g, at(101), &wrong),
        Err(Error::Guard(UploadError::DigestMismatch))
    ));
    store.stage(&i, r, g, at(101), PDF).unwrap();
    fs::write(path(&root, b, 1, "stage"), wrong).unwrap();
    assert!(matches!(
        store.finalize(&mut i, r, g, at(102)),
        Err(Error::Guard(UploadError::DigestMismatch))
    ));
    assert_eq!(i.state(), IntentState::Pending);
    assert!(!path(&root, b, 1, "quarantine").exists());
    let mut other = create(binding(10, 21, 30), b"not a pdf");
    let grant = other.issue_grant(r, at(100), 60).unwrap();
    assert!(matches!(
        store.stage(&other, r, grant, at(101), b"not a pdf"),
        Err(Error::MimeMismatch)
    ));
}
#[test]
fn metadata_quota_and_office_intent_gate() {
    let b = binding(10, 20, 30);
    let r = revision(b);
    for n in ["../a.pdf", "a/b.pdf", "a\\b.pdf", "a\0.pdf"] {
        assert!(matches!(
            Intent::create(b, r, n, 1, digest(b"x"), DeclaredMime::Pdf, 0),
            Err(Error::InvalidFilename)
        ));
    }
    assert!(matches!(
        Intent::create(
            b,
            r,
            "a.pdf",
            DEFAULT_MAX_FILE_BYTES + 1,
            digest(PDF),
            DeclaredMime::Pdf,
            0
        ),
        Err(Error::Guard(UploadError::InvalidSize))
    ));
    assert!(matches!(
        Intent::create(
            b,
            r,
            "a.pdf",
            1,
            digest(PDF),
            DeclaredMime::Pdf,
            DEFAULT_MAX_DOSSIER_BYTES
        ),
        Err(Error::Guard(UploadError::QuotaExceeded))
    ));
    assert!(
        Intent::create(b, r, "a.docx", 1, digest(PDF), DeclaredMime::Docx, 0).is_ok(),
        "Office metadata intent is allowed; ZIP signature and quarantine/scan gates apply at byte ingestion"
    );
    let mut i = create(b, PDF);
    assert!(i.issue_grant(r, at(100), 301).is_err());
}
