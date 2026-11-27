use finance_documents::{
    Availability, Binding, Original,
    scan::{ClamdScanner, ScanError, ScanVerdict},
};
use finance_shared::{
    contracts::Version,
    ids::{CompanyId, CompanyScope, ResourceId, RevisionId, TenantId},
    upload_contract::{DeclaredMime, Sha256Digest},
};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    thread,
    time::Duration,
};
use uuid::Uuid;

const CLEAN: &[u8] = b"%PDF-1.7\nSynthetic harmless document\n";
const MATCHED: &[u8] = b"%PDF-1.7\nFINANCE_SYNTHETIC_SCAN_PROBE_2026\n";

fn original(bytes: &[u8]) -> Original {
    Original {
        binding: Binding {
            scope: CompanyScope {
                tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
                company: CompanyId::try_from(Uuid::from_u128(10)).unwrap(),
            },
            document: ResourceId::try_from(Uuid::from_u128(20)).unwrap(),
            revision: RevisionId::try_from(Uuid::from_u128(30)).unwrap(),
        },
        bytes: bytes.len() as u64,
        digest: Sha256Digest::parse(&format!("{:x}", Sha256::digest(bytes))).unwrap(),
        mime: DeclaredMime::Pdf,
        generation: Version::try_from(1).unwrap(),
        availability: Availability::Quarantined,
    }
}

#[test]
fn transport_framing_and_original_binding() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let scanner = ClamdScanner::new(listener.local_addr().unwrap(), Duration::from_secs(3)).unwrap();
    let fixture = original(CLEAN);
    assert_eq!(
        scanner.scan(fixture, MATCHED),
        Err(ScanError::InvalidOriginal)
    );
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut command = [0_u8; 10];
        socket.read_exact(&mut command).unwrap();
        assert_eq!(&command, b"zINSTREAM\0");
        let mut payload = Vec::new();
        loop {
            let mut length = [0_u8; 4];
            socket.read_exact(&mut length).unwrap();
            let length = u32::from_be_bytes(length) as usize;
            if length == 0 {
                break;
            }
            assert!(length <= 8192);
            let mut chunk = vec![0_u8; length];
            socket.read_exact(&mut chunk).unwrap();
            payload.extend(chunk);
        }
        assert_eq!(payload, CLEAN);
        socket.write_all(b"stream: OK\0").unwrap();
    });
    let receipt = scanner.scan(fixture, CLEAN).unwrap();
    assert_eq!(receipt.original(), fixture);
    assert_eq!(receipt.verdict(), ScanVerdict::NoSignatureMatch);
    assert_eq!(receipt.original().availability, Availability::Quarantined);
    server.join().unwrap();
}

#[test]
fn no_response_times_out_without_clean_verdict() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let scanner = ClamdScanner::new(
        listener.local_addr().unwrap(),
        Duration::from_millis(150),
    )
    .unwrap();
    let server = thread::spawn(move || {
        let (_socket, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(400));
    });
    assert_eq!(scanner.scan(original(CLEAN), CLEAN), Err(ScanError::Timeout));
    server.join().unwrap();
}

#[test]
#[ignore = "requires disposable real clamd; scripts/test_clamd_probe.sh"]
fn real_clamd_synthetic_signatures() {
    let endpoint: SocketAddr = std::env::var("FINANCE_CLAMD_PROBE_ADDRESS")
        .expect("disposable scanner address")
        .parse()
        .unwrap();
    let scanner = ClamdScanner::new(endpoint, Duration::from_secs(15)).unwrap();
    let clean = scanner.scan(original(CLEAN), CLEAN).unwrap();
    let matched = scanner.scan(original(MATCHED), MATCHED).unwrap();
    assert_eq!(clean.verdict(), ScanVerdict::NoSignatureMatch);
    assert_eq!(matched.verdict(), ScanVerdict::SignatureDetected);
    assert_eq!(clean.original().availability, Availability::Quarantined);
    assert_eq!(matched.original().availability, Availability::Quarantined);
}
