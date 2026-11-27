#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
pub mod repository;
use finance_shared::upload_contract::Sha256Digest;
use std::collections::BTreeSet;

pub struct Artifact<'a> {
    pub name: &'a str,
    pub expected: Sha256Digest,
    pub observed: Option<Sha256Digest>,
    pub bytes: u64,
}

/// Manifest guard only. Observed hash must originate in trusted storage
/// readback; this does not render or store a file.
pub fn verified_manifest(artifacts: &[Artifact<'_>]) -> bool {
    if artifacts.is_empty() || artifacts.len() > 256 {
        return false;
    }
    let mut names = BTreeSet::new();
    artifacts.iter().all(|a| {
        a.bytes > 0
            && a.bytes <= 100 * 1024 * 1024
            && a.observed == Some(a.expected)
            && !a.name.is_empty()
            && a.name.len() <= 128
            && !a.name.starts_with('.')
            && a.name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            && names.insert(a.name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_mismatched_readback_denied() {
        let d = Sha256Digest::parse(&"a".repeat(64)).unwrap();
        assert!(!verified_manifest(&[]));
        assert!(!verified_manifest(&[Artifact {
            name: "a.pdf",
            expected: d,
            observed: None,
            bytes: 1
        }]));
        assert!(verified_manifest(&[Artifact {
            name: "a.pdf",
            expected: d,
            observed: Some(d),
            bytes: 1
        }]));
        assert!(!verified_manifest(&[Artifact {
            name: "../a.pdf",
            expected: d,
            observed: Some(d),
            bytes: 1
        }]));
    }
}
