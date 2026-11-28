//! Independent local artifact readback for the configured export adapter.
//! All path components below the private root are typed IDs or validated
//! names. The root must be owned by the isolated renderer/worker trust domain;
//! never point it at a directory writable by users or other applications.
use super::{Lease, OwnedArtifact, RunFailure};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

const MAX_TOTAL: u64 = 512 * 1024 * 1024;

fn private_directory(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return false;
        }
    }
    true
}

pub(super) fn valid_root(root: &Path) -> bool {
    root.is_absolute()
        && private_directory(root)
        && root.canonicalize().is_ok_and(|path| path == root)
}

fn job_directory(root: &Path, lease: Lease) -> Result<PathBuf, RunFailure> {
    if !valid_root(root) || lease.fence < 1 {
        return Err(RunFailure::Permanent);
    }
    let scope = lease.scope();
    let mut path = root.to_path_buf();
    for component in [
        scope.tenant.to_string(),
        scope.company.to_string(),
        lease.job_id.to_string(),
        lease.fence.to_string(),
    ] {
        path.push(component);
        if !private_directory(&path) {
            return Err(RunFailure::Permanent);
        }
    }
    Ok(path)
}

fn verified_file(directory: &Path, item: &OwnedArtifact) -> Result<(), RunFailure> {
    if item.name.is_empty()
        || item.name.len() > 128
        || item.name.starts_with('.')
        || !item
            .name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        || item.bytes == 0
        || item.bytes > 100 * 1024 * 1024
    {
        return Err(RunFailure::Permanent);
    }
    let path = directory.join(&item.name);
    let metadata = fs::symlink_metadata(&path).map_err(|_| RunFailure::Permanent)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != item.bytes {
        return Err(RunFailure::Permanent);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 {
            return Err(RunFailure::Permanent);
        }
    }
    let file = File::open(&path).map_err(|_| RunFailure::Transient)?;
    let mut reader = file.take(item.bytes + 1);
    let mut total = 0_u64;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let n = reader.read(&mut buffer).map_err(|_| RunFailure::Transient)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hash.update(&buffer[..n]);
    }
    let observed: [u8; 32] = hash.finalize().into();
    if total != item.bytes || observed != item.expected.as_bytes() || observed != item.observed.as_bytes() {
        return Err(RunFailure::Permanent);
    }
    Ok(())
}

pub(super) fn verify(root: &Path, lease: Lease, artifacts: &[OwnedArtifact]) -> Result<(), RunFailure> {
    let total = artifacts.iter().try_fold(0_u64, |total, item| total.checked_add(item.bytes));
    if artifacts.is_empty() || artifacts.len() > 256 || total.is_none_or(|n| n > MAX_TOTAL) {
        return Err(RunFailure::Permanent);
    }
    let directory = job_directory(root, lease)?;
    for artifact in artifacts {
        verified_file(&directory, artifact)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use finance_shared::upload_contract::Sha256Digest;
    use uuid::Uuid;

    #[test]
    #[cfg(unix)]
    fn rehashes_real_bytes_and_rejects_paths_and_tampering() {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
        use std::io::Write;

        let root = std::env::temp_dir().join(format!("finance-artifact-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let payload = b"%PDF-1.7 synthetic";
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join("request.pdf"))
            .unwrap();
        f.write_all(payload).unwrap();
        drop(f);
        let digest = Sha256Digest::parse(&format!("{:x}", Sha256::digest(payload))).unwrap();
        let mut artifact = OwnedArtifact {
            name: "request.pdf".into(),
            expected: digest,
            observed: digest,
            bytes: payload.len() as u64,
        };
        assert_eq!(verified_file(&root, &artifact), Ok(()));
        fs::write(root.join("request.pdf"), b"%PDF-1.7 tampered!").unwrap();
        assert_eq!(verified_file(&root, &artifact), Err(RunFailure::Permanent));
        symlink(root.join("request.pdf"), root.join("link.pdf")).unwrap();
        artifact.name = "link.pdf".into();
        assert_eq!(verified_file(&root, &artifact), Err(RunFailure::Permanent));
        artifact.name = "../request.pdf".into();
        assert_eq!(verified_file(&root, &artifact), Err(RunFailure::Permanent));
        fs::remove_dir_all(root).unwrap();
    }
}
