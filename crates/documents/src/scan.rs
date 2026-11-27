//! Isolated ClamAV INSTREAM transport for synthetic CI only.
//! The scanner never grants CLEAN or permits download/OCR.
use crate::{Availability, Original, hash};
use finance_shared::{
    ids::CompanyScope,
    upload_contract::{DEFAULT_MAX_FILE_BYTES, Sha256Digest},
};
use sqlx::PgPool;
use uuid::Uuid;
use std::{
    io::{Read, Write},
    net::{IpAddr, SocketAddr, TcpStream},
    time::{Duration, Instant},
};

const MAX_RESPONSE: usize = 512;
const CHUNK_SIZE: usize = 8192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanVerdict {
    NoSignatureMatch,
    SignatureDetected,
    Indeterminate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanError {
    InvalidConfiguration,
    InvalidOriginal,
    Timeout,
    Transport,
    InvalidResponse,
}

/// A scan result is bound to the exact original metadata and bytes, never a
/// standalone boolean that can release another document from quarantine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanReceipt {
    original: Original,
    verdict: ScanVerdict,
}
impl ScanReceipt {
    pub fn original(&self) -> Original {
        self.original
    }

    pub fn verdict(&self) -> ScanVerdict {
        self.verdict
    }
}

/// Restrict prototype to a dedicated local clamd. Production transport and
/// authenticated worker isolation require separate approval.
pub struct ClamdScanner {
    address: SocketAddr,
    timeout: Duration,
}

impl ClamdScanner {
    pub fn new(address: SocketAddr, timeout: Duration) -> Result<Self, ScanError> {
        if !matches!(address.ip(), IpAddr::V4(ip) if ip.is_loopback())
            || address.port() == 0
            || timeout.is_zero()
            || timeout > Duration::from_secs(60)
        {
            return Err(ScanError::InvalidConfiguration);
        }
        Ok(Self { address, timeout })
    }

    /// Blocking I/O; invoke only from an isolated bounded worker, never a web
    /// async reactor. Hash verification precedes the scan. No raw payload,
    /// scanner reply, path, or signature name is logged.
    pub fn scan(&self, original: Original, bytes: &[u8]) -> Result<ScanReceipt, ScanError> {
        if original.availability != Availability::Quarantined
            || original.bytes == 0
            || original.bytes > DEFAULT_MAX_FILE_BYTES
            || bytes.len() as u64 != original.bytes
            || hash(bytes).map_err(|_| ScanError::InvalidOriginal)? != original.digest
        {
            return Err(ScanError::InvalidOriginal);
        }
        let deadline = Instant::now() + self.timeout;
        let mut stream =
            TcpStream::connect_timeout(&self.address, remaining(deadline)?).map_err(map_io)?;
        write_bounded(&mut stream, b"zINSTREAM\0", deadline)?;
        for chunk in bytes.chunks(CHUNK_SIZE) {
            let length = u32::try_from(chunk.len()).map_err(|_| ScanError::InvalidOriginal)?;
            write_bounded(&mut stream, &length.to_be_bytes(), deadline)?;
            write_bounded(&mut stream, chunk, deadline)?;
        }
        write_bounded(&mut stream, &0_u32.to_be_bytes(), deadline)?;
        let reply = read_reply(&mut stream, deadline)?;
        let verdict = parse_reply(&reply)?;
        Ok(ScanReceipt { original, verdict })
    }
}

fn remaining(deadline: Instant) -> Result<Duration, ScanError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or(ScanError::Timeout)
}

fn map_io(error: std::io::Error) -> ScanError {
    match error.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => ScanError::Timeout,
        _ => ScanError::Transport,
    }
}

fn write_bounded(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> Result<(), ScanError> {
    while !bytes.is_empty() {
        stream
            .set_write_timeout(Some(remaining(deadline)?))
            .map_err(map_io)?;
        match stream.write(bytes) {
            Ok(0) => return Err(ScanError::Transport),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(map_io(e)),
        }
    }
    Ok(())
}

fn read_reply(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>, ScanError> {
    let mut reply = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(map_io)?;
        match stream.read(&mut byte) {
            Ok(0) => return Err(ScanError::InvalidResponse),
            Ok(_) if byte[0] == 0 => return Ok(reply),
            Ok(_) if reply.len() < MAX_RESPONSE => reply.push(byte[0]),
            Ok(_) => return Err(ScanError::InvalidResponse),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(map_io(e)),
        }
    }
}

fn parse_reply(reply: &[u8]) -> Result<ScanVerdict, ScanError> {
    let text = std::str::from_utf8(reply).map_err(|_| ScanError::InvalidResponse)?;
    if !text.bytes().all(|b| (32..=126).contains(&b)) {
        return Err(ScanError::InvalidResponse);
    }
    if text == "stream: OK" {
        return Ok(ScanVerdict::NoSignatureMatch);
    }
    if let Some(signature) = text
        .strip_prefix("stream: ")
        .and_then(|s| s.strip_suffix(" FOUND"))
        .filter(|s| !s.is_empty())
    {
        if signature == "OK" {
            return Err(ScanError::InvalidResponse);
        }
        return Ok(ScanVerdict::SignatureDetected);
    }
    if text.ends_with(" ERROR") {
        return Ok(ScanVerdict::Indeterminate);
    }
    Err(ScanError::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanner_reply_is_strict_and_fail_closed() {
        assert_eq!(
            parse_reply(b"stream: OK"),
            Ok(ScanVerdict::NoSignatureMatch)
        );
        assert_eq!(
            parse_reply(b"stream: SyntheticProbe.UNOFFICIAL FOUND"),
            Ok(ScanVerdict::SignatureDetected)
        );
        assert_eq!(
            parse_reply(b"INSTREAM size limit exceeded. ERROR"),
            Ok(ScanVerdict::Indeterminate)
        );
        for bad in [
            b"OK".as_slice(),
            b"stream:  FOUND",
            b"stream: OK FOUND",
            b"stream: OK\nother",
            b"stream: OK ERROR\r",
            b"stream: \xff",
            b"",
        ] {
            assert_eq!(parse_reply(bad), Err(ScanError::InvalidResponse));
        }
    }

    #[test]
    fn scanner_endpoint_is_local_and_bounded() {
        assert!(
            ClamdScanner::new("127.0.0.1:3310".parse().unwrap(), Duration::from_secs(1)).is_ok()
        );
        assert!(
            ClamdScanner::new("192.0.2.1:3310".parse().unwrap(), Duration::from_secs(1)).is_err()
        );
        assert!(ClamdScanner::new("127.0.0.1:3310".parse().unwrap(), Duration::ZERO).is_err());
        assert!(
            ClamdScanner::new("127.0.0.1:3310".parse().unwrap(), Duration::from_secs(61)).is_err()
        );
    }
}


#[derive(Debug)]
pub enum PersistError {
    Forbidden,
    InvalidInput,
    StaleFence,
    Database(sqlx::Error),
}
impl From<sqlx::Error> for PersistError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

/// Persist a determinate ClamAV verdict only when the worker still owns the
/// active fenced job that was dispatched for this exact document resource.
pub async fn persist_verdict(
    pool: &PgPool,
    scope: CompanyScope,
    job: Uuid,
    fence: i64,
    document: Uuid,
    verdict: ScanVerdict,
    engine_sha256: &str,
) -> Result<Availability, PersistError> {
    if job.is_nil()
        || document.is_nil()
        || fence < 1
        || Sha256Digest::parse(engine_sha256).is_err()
        || verdict == ScanVerdict::Indeterminate
    {
        return Err(PersistError::InvalidInput);
    }
    let (t, c): (Uuid, Uuid) = (scope.tenant.into(), scope.company.into());
    let mut tx = pool.begin().await?;
    let role: bool = sqlx::query_scalar(
        "SELECT current_user='finance_job_worker' AND NOT rolsuper AND NOT rolbypassrls FROM pg_roles WHERE rolname=current_user",
    )
    .fetch_one(&mut *tx)
    .await?;
    if !role {
        return Err(PersistError::Forbidden);
    }
    sqlx::query(
        "SELECT set_config('finance.tenant_id',$1,true),set_config('finance.company_id',$2,true)",
    )
    .bind(t.to_string())
    .bind(c.to_string())
    .execute(&mut *tx)
    .await?;
    let allowed: bool = sqlx::query_scalar("SELECT finance.worker_scope_allowed($1,$2)")
        .bind(t)
        .bind(c)
        .fetch_one(&mut *tx)
        .await?;
    if !allowed {
        return Err(PersistError::Forbidden);
    }
    let owns: bool = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM finance.job j
           JOIN finance.job_attempt a ON (a.tenant_id,a.company_id,a.job_id,a.fence)=(j.tenant_id,j.company_id,j.id,j.fence)
           JOIN finance.outbox o ON (o.tenant_id,o.company_id,o.id)=(j.tenant_id,j.company_id,j.id)
           WHERE j.tenant_id=$1 AND j.company_id=$2 AND j.id=$3 AND j.fence=$4
           AND j.state='LEASED' AND j.lease_until>clock_timestamp()
           AND a.state='ACTIVE' AND o.resource_id=$5)",
    )
    .bind(t)
    .bind(c)
    .bind(job)
    .bind(fence)
    .bind(document)
    .fetch_one(&mut *tx)
    .await?;
    if !owns {
        return Err(PersistError::StaleFence);
    }
    let (state, availability) = match verdict {
        ScanVerdict::NoSignatureMatch => ("CLEAN", Availability::Clean),
        ScanVerdict::SignatureDetected => ("REJECTED", Availability::Rejected),
        ScanVerdict::Indeterminate => unreachable!(),
    };
    let changed = sqlx::query(
        "UPDATE finance.document_blob SET scan_state=$6,scan_engine='CLAMAV',scan_engine_digest=decode($7,'hex'),scanned_at=clock_timestamp()
         WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND scan_state='QUARANTINED'
         AND EXISTS(SELECT 1 FROM finance.document_ref r WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.id=$3)",
    )
    .bind(t)
    .bind(c)
    .bind(document)
    .bind(job)
    .bind(fence)
    .bind(state)
    .bind(engine_sha256)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(PersistError::StaleFence);
    }
    tx.commit().await?;
    Ok(availability)
}
