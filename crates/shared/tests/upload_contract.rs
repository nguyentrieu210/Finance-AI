use chrono::{DateTime, Utc};
use finance_shared::{contracts::Version, upload_contract::*};
fn v(n: i64) -> Version {
    Version::try_from(n).unwrap()
}
fn time(n: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(n, 0).unwrap()
}
fn digest(ch: char) -> Sha256Digest {
    Sha256Digest::parse(&ch.to_string().repeat(64)).unwrap()
}
#[test]
fn metadata_limits_and_checksum_are_fail_closed() {
    assert!(Sha256Digest::parse(&"A".repeat(64)).is_err());
    assert!(Sha256Digest::parse("abcd").is_err());
    assert!(DeclaredMime::parse("application/zip").is_err());
    let mime = DeclaredMime::parse("application/pdf").unwrap();
    assert_eq!(
        UploadMetadata::new(0, digest('a'), mime, DEFAULT_MAX_FILE_BYTES),
        Err(UploadError::InvalidSize)
    );
    assert!(
        UploadMetadata::new(
            DEFAULT_MAX_FILE_BYTES + 1,
            digest('a'),
            mime,
            DEFAULT_MAX_FILE_BYTES
        )
        .is_err()
    );
    let m = UploadMetadata::new(
        DEFAULT_MAX_FILE_BYTES,
        digest('a'),
        mime,
        DEFAULT_MAX_FILE_BYTES,
    )
    .unwrap();
    assert_eq!(
        m.check_quota(
            DEFAULT_MAX_DOSSIER_BYTES - DEFAULT_MAX_FILE_BYTES,
            DEFAULT_MAX_DOSSIER_BYTES
        ),
        Ok(())
    );
    assert_eq!(
        m.check_quota(u64::MAX, DEFAULT_MAX_DOSSIER_BYTES),
        Err(UploadError::QuotaExceeded)
    );
    assert_eq!(
        m.verify_bytes(DEFAULT_MAX_FILE_BYTES - 1, digest('a')),
        Err(UploadError::ByteMismatch)
    );
    assert_eq!(
        m.verify_bytes(DEFAULT_MAX_FILE_BYTES, digest('b')),
        Err(UploadError::DigestMismatch)
    );
    assert_eq!(m.verify_bytes(DEFAULT_MAX_FILE_BYTES, digest('a')), Ok(()));
}
#[test]
fn refresh_fences_old_grants_and_expiry() {
    let first = UploadIntentGuard {
        state: IntentState::Pending,
        generation: v(0),
    };
    let grant1 = first.next_grant(time(10), time(20)).unwrap();
    let current = UploadIntentGuard {
        generation: grant1.generation,
        ..first
    };
    let grant2 = current.next_grant(time(11), time(21)).unwrap();
    let refreshed = UploadIntentGuard {
        generation: grant2.generation,
        ..first
    };
    assert_eq!(
        refreshed.check_finalize(v(1), grant1, time(12)),
        Err(UploadError::StaleGrant)
    );
    assert_eq!(
        refreshed.check_finalize(v(2), grant2, time(21)),
        Err(UploadError::GrantExpired)
    );
    assert_eq!(refreshed.check_finalize(v(2), grant2, time(20)), Ok(()));
    assert_eq!(
        first.next_grant(time(10), time(10)),
        Err(UploadError::InvalidExpiry)
    );
}
#[test]
fn terminal_intents_and_overflow_cannot_issue_grants() {
    for state in [IntentState::Finalized, IntentState::Cancelled] {
        let guard = UploadIntentGuard {
            state,
            generation: v(1),
        };
        assert_eq!(
            guard.next_grant(time(10), time(20)),
            Err(UploadError::Closed)
        );
        assert_eq!(
            guard.check_finalize(
                v(1),
                GrantFence {
                    generation: v(1),
                    expires_at: time(20)
                },
                time(11)
            ),
            Err(UploadError::Closed)
        );
    }
    let max = UploadIntentGuard {
        state: IntentState::Pending,
        generation: v(9_007_199_254_740_991),
    };
    assert_eq!(
        max.next_grant(time(10), time(20)),
        Err(UploadError::VersionOverflow)
    );
}
