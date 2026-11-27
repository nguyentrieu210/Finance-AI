#![forbid(unsafe_code)]
//! Synthetic prototype audit read and offline integrity verification. No public API.
use finance_core::prototype::{Error, scoped_transaction};
use finance_shared::{
    contracts::{CommandContext, Principal},
    ids::ResourceScope,
};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Event {
    pub tenant: Uuid,
    pub company: Uuid,
    pub id: Uuid,
    pub actor: Uuid,
    pub command: Uuid,
    pub correlation: Uuid,
    pub resource: Uuid,
    pub sequence: i64,
    pub unix_micros: i64,
    pub action: String,
    pub previous: [u8; 32],
    pub digest: [u8; 32],
}
impl Event {
    pub fn calculate_digest(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"finance.audit/v1");
        h.update(self.previous);
        for id in [
            self.tenant,
            self.company,
            self.id,
            self.actor,
            self.command,
            self.correlation,
            self.resource,
        ] {
            h.update(id.as_bytes());
        }
        h.update(self.sequence.to_be_bytes());
        h.update(self.unix_micros.to_be_bytes());
        h.update(self.action.as_bytes());
        h.finalize().into()
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Head {
    pub sequence: i64,
    pub digest: [u8; 32],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegrityError {
    WrongTenant,
    Gap,
    PreviousMismatch,
    DigestMismatch,
    HeadMismatch,
}
/// Verify a full tenant chain, not a filtered company page. Trusted independent head required to detect full-history rewrite.
pub fn verify_chain(
    tenant: Uuid,
    events: impl IntoIterator<Item = Event>,
    expected: Head,
) -> Result<(), IntegrityError> {
    let mut head = Head {
        sequence: 0,
        digest: [0; 32],
    };
    for event in events {
        if event.tenant != tenant {
            return Err(IntegrityError::WrongTenant);
        }
        if head.sequence.checked_add(1) != Some(event.sequence) {
            return Err(IntegrityError::Gap);
        }
        if event.previous != head.digest {
            return Err(IntegrityError::PreviousMismatch);
        }
        if event.calculate_digest() != event.digest {
            return Err(IntegrityError::DigestMismatch);
        }
        head = Head {
            sequence: event.sequence,
            digest: event.digest,
        };
    }
    if head != expected {
        return Err(IntegrityError::HeadMismatch);
    }
    Ok(())
}
pub fn decode_event(row: &sqlx::postgres::PgRow) -> Result<Event, sqlx::Error> {
    fn digest(value: Vec<u8>) -> Result<[u8; 32], sqlx::Error> {
        value
            .try_into()
            .map_err(|_| sqlx::Error::Protocol("invalid audit digest length".into()))
    }
    Ok(Event {
        tenant: row.try_get("tenant_id")?,
        company: row.try_get("company_id")?,
        id: row.try_get("id")?,
        actor: row.try_get("actor_id")?,
        command: row.try_get("command_id")?,
        correlation: row.try_get("correlation_id")?,
        resource: row.try_get("resource_id")?,
        sequence: row.try_get("sequence")?,
        unix_micros: row.try_get("unix_micros")?,
        action: row.try_get("action")?,
        previous: digest(row.try_get("previous_digest")?)?,
        digest: digest(row.try_get("digest")?)?,
    })
}
pub const EVENT_FIELDS: &str = "tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,sequence,(extract(epoch FROM occurred_at)*1000000)::bigint AS unix_micros,action,previous_digest,digest";
/// Scoped auditor page; inserts read access event in same transaction before returning.
/// Raw sequence pagination is internal prototype only, not the public cursor contract.
pub async fn read_page(
    pool: &PgPool,
    ctx: &CommandContext,
    after: i64,
    limit: i64,
) -> Result<Vec<Event>, Error> {
    if after < 0 || !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let ResourceScope::Company(scope) = ctx.scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(actor) = ctx.actor else {
        return Err(Error::Forbidden);
    };
    let (t, c, a): (Uuid, Uuid, Uuid) = (scope.tenant.into(), scope.company.into(), actor.into());
    let mut tx = scoped_transaction(pool, ctx).await?;
    let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM finance.membership m JOIN finance.identity_subject i ON (i.tenant_id,i.id)=(m.tenant_id,m.actor_id) JOIN finance.company c ON (c.tenant_id,c.id)=(m.tenant_id,m.company_id) JOIN finance.tenant t ON t.id=m.tenant_id WHERE m.tenant_id=$1 AND m.company_id=$2 AND m.actor_id=$3 AND m.role='AUDITOR' AND m.active AND i.active AND c.active AND t.active AND m.valid_from<=statement_timestamp() AND (m.valid_until IS NULL OR m.valid_until>statement_timestamp()))")
        .bind(t).bind(c).bind(a).fetch_one(&mut *tx).await?;
    if !allowed {
        return Err(Error::Forbidden);
    }
    let sql = format!(
        "SELECT {EVENT_FIELDS} FROM finance.audit_event WHERE tenant_id=$1 AND company_id=$2 AND sequence>$3 AND action<>'AUDIT_READ' ORDER BY sequence LIMIT $4"
    );
    // Exclude read-access events from this page to prevent endless pagination feedback.
    // Separate access-audit export remains future work.
    let rows = sqlx::query(&sql)
        .bind(t)
        .bind(c)
        .bind(after)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
    let events = rows
        .iter()
        .map(decode_event)
        .collect::<Result<Vec<_>, _>>()?;
    let cmd: Uuid = ctx.command.into();
    let corr: Uuid = ctx.correlation.into();
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$2,'AUDIT_READ')")
        .bind(t).bind(c).bind(cmd).bind(a).bind(corr).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(events)
}
