//! E21 authenticated risk and waiver repository.
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, lock_roles, require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
    ids::{CompanyScope, ResourceScope},
};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, sqlx::FromRow)]
pub struct Risk {
    pub id: Uuid,
    pub snapshot_id: Uuid,
    pub code: String,
    pub severity: String,
    pub state: String,
    pub evidence_ids: Vec<Uuid>,
}
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, sqlx::FromRow)]
pub struct Waiver {
    pub id: Uuid,
    pub result_id: Uuid,
    pub snapshot_id: Option<Uuid>,
    pub state: String,
    pub reason: String,
    pub expires_at: String,
    pub evidence_ids: Vec<Uuid>,
    pub input_hash: String,
    pub revision: i64,
    #[serde(skip)]
    pub dossier_id: Uuid,
    #[serde(skip)]
    pub dossier_version: i64,
}

pub struct WaiverRequestCommand<'a> {
    pub dossier: Uuid,
    pub id: Uuid,
    pub result: Uuid,
    pub reason: &'a str,
    pub evidence: &'a [Uuid],
}

pub struct WaiverDecisionCommand<'a> {
    pub dossier: Uuid,
    pub waiver: Uuid,
    pub decision: &'a str,
    pub expires: Option<chrono::DateTime<chrono::Utc>>,
    pub reason: &'a str,
    pub evidence: &'a [Uuid],
}

fn binding(proof: &AuthenticatedCommand) -> Result<(Uuid, Uuid, Uuid), Error> {
    let ResourceScope::Company(CompanyScope { tenant, company }) = proof.context().scope else {
        return Err(Error::Forbidden);
    };
    let Principal::User(actor) = proof.context().actor else {
        return Err(Error::Forbidden);
    };
    Ok((tenant.into(), company.into(), actor.into()))
}
fn grant(e: GrantError) -> Error {
    match e {
        GrantError::Forbidden => Error::Forbidden,
        GrantError::Database(e) => Error::Database(e),
    }
}
async fn risk_read(
    tx: &mut Transaction<'_, Postgres>,
    proof: &AuthenticatedCommand,
) -> Result<(), Error> {
    require_role(tx, proof.context(), Role::Requester)
        .await
        .map_err(grant)
}
async fn waiver_read(
    tx: &mut Transaction<'_, Postgres>,
    proof: &AuthenticatedCommand,
) -> Result<(), Error> {
    let roles = lock_roles(tx, proof.context()).await.map_err(grant)?;
    if roles.contains(&Role::Requester) || roles.contains(&Role::FinanceApprover) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}
fn evidence_reason(reason: &str, evidence: &[Uuid]) -> Result<(), Error> {
    if reason.trim().is_empty()
        || reason.len() > 4096
        || reason.chars().any(char::is_control)
        || evidence.len() > 64
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}
fn hash(parts: &[&[u8]]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(b"finance.waiver.command/v1");
    for p in parts {
        h.update((p.len() as u64).to_be_bytes());
        h.update(p);
    }
    h.finalize().to_vec()
}
const SELECT_WAIVER: &str = "SELECT w.id,w.result_id,s.id AS snapshot_id,CASE WHEN w.state='APPROVED' AND w.expires_at<=clock_timestamp() THEN 'EXPIRED' ELSE w.state END AS state,COALESCE(w.decision_reason,w.reason) AS reason,to_char(w.expires_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS expires_at,CASE WHEN cardinality(w.decision_evidence_ids)>0 THEN w.decision_evidence_ids ELSE w.evidence_ids END AS evidence_ids,encode(w.input_hash,'hex') AS input_hash,dr.number AS revision,w.dossier_id,d.version AS dossier_version FROM finance.waiver w JOIN finance.dossier d ON (d.tenant_id,d.company_id,d.id)=(w.tenant_id,w.company_id,w.dossier_id) JOIN finance.dossier_revision dr ON (dr.tenant_id,dr.company_id,dr.id)=(w.tenant_id,w.company_id,w.revision_id) LEFT JOIN finance.dossier_snapshot s ON (s.tenant_id,s.company_id,s.dossier_id,s.revision_id)=(w.tenant_id,w.company_id,w.dossier_id,w.revision_id)";

async fn verify_evidence(
    tx: &mut Transaction<'_, Postgres>,
    t: Uuid,
    c: Uuid,
    dossier: Uuid,
    revision: Uuid,
    evidence: &[Uuid],
) -> Result<bool, Error> {
    if evidence.is_empty() {
        return Ok(true);
    }
    let count:i64=sqlx::query_scalar("SELECT count(DISTINCT e.id)::bigint FROM finance.evidence e JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.id)=(e.tenant_id,e.company_id,e.ocr_run_id) JOIN finance.document_ref r ON (r.tenant_id,r.company_id,r.id)=(o.tenant_id,o.company_id,o.document_id) WHERE e.tenant_id=$1 AND e.company_id=$2 AND e.id=ANY($3) AND r.dossier_id=$4 AND r.revision_id=$5")
        .bind(t).bind(c).bind(evidence).bind(dossier).bind(revision).fetch_one(&mut **tx).await?;
    Ok(count == evidence.len() as i64)
}

pub async fn list_risks(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Risk>, Error> {
    if !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (t, c, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    risk_read(&mut tx, proof).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows=sqlx::query_as::<_,Risk>("SELECT s.id,s.snapshot_id,s.code,s.severity,CASE WHEN d.resolution='ESCALATED' THEN 'ESCALATED' WHEN d.id IS NULL THEN 'OPEN' ELSE 'RESOLVED' END AS state,s.evidence_ids FROM finance.risk_signal s LEFT JOIN finance.risk_decision d ON (d.tenant_id,d.company_id,d.signal_id)=(s.tenant_id,s.company_id,s.id) WHERE s.tenant_id=$1 AND s.company_id=$2 AND s.id>$3 ORDER BY s.id LIMIT $4")
        .bind(t).bind(c).bind(after).bind(limit).fetch_all(&mut *tx).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}
pub async fn get_risk(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    id: Uuid,
) -> Result<Option<Risk>, Error> {
    if id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (t, c, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    risk_read(&mut tx, proof).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row=sqlx::query_as::<_,Risk>("SELECT s.id,s.snapshot_id,s.code,s.severity,CASE WHEN d.resolution='ESCALATED' THEN 'ESCALATED' WHEN d.id IS NULL THEN 'OPEN' ELSE 'RESOLVED' END AS state,s.evidence_ids FROM finance.risk_signal s LEFT JOIN finance.risk_decision d ON (d.tenant_id,d.company_id,d.signal_id)=(s.tenant_id,s.company_id,s.id) WHERE s.tenant_id=$1 AND s.company_id=$2 AND s.id=$3")
        .bind(t).bind(c).bind(id).fetch_optional(&mut *tx).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
pub async fn list_waivers(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    dossier: Uuid,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Waiver>, Error> {
    if dossier.is_nil() || !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (t, c, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    waiver_read(&mut tx, proof).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows=sqlx::query_as::<_,Waiver>(&format!("{SELECT_WAIVER} WHERE w.tenant_id=$1 AND w.company_id=$2 AND w.dossier_id=$3 AND w.id>$4 ORDER BY w.id LIMIT $5"))
        .bind(t).bind(c).bind(dossier).bind(after).bind(limit).fetch_all(&mut *tx).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}
pub async fn request_waiver(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    command: WaiverRequestCommand<'_>,
) -> Result<Waiver, Error> {
    let WaiverRequestCommand {
        dossier,
        id,
        result,
        reason,
        evidence,
    } = command;
    evidence_reason(reason, evidence)?;
    let Some(expected) = proof.context().expected_version else {
        return Err(Error::InvalidInput);
    };
    let expected = i64::from(expected);
    if dossier.is_nil() || id.is_nil() || result.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::Requester)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/waiver.request/{}", key.as_str()))
        .execute(&mut *tx)
        .await?;
    let mut ev = evidence.to_vec();
    ev.sort();
    ev.dedup();
    if ev.len() != evidence.len() {
        return Err(Error::InvalidInput);
    }
    let expected_bytes = expected.to_be_bytes();
    let mut ev_bytes = Vec::with_capacity(ev.len() * 16);
    for e in &ev {
        ev_bytes.extend_from_slice(e.as_bytes())
    }
    let digest = hash(&[
        dossier.as_bytes(),
        result.as_bytes(),
        &expected_bytes,
        reason.as_bytes(),
        &ev_bytes,
    ]);
    let prior:Option<(Vec<u8>,Uuid)>=sqlx::query_as("SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='waiver.request' AND key=$4")
        .bind(t).bind(c).bind(a).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((old, wid)) = prior {
        if old != digest {
            return Err(Error::Conflict);
        }
        let row = sqlx::query_as::<_, Waiver>(&format!(
            "{SELECT_WAIVER} WHERE w.tenant_id=$1 AND w.company_id=$2 AND w.id=$3"
        ))
        .bind(t)
        .bind(c)
        .bind(wid)
        .fetch_one(&mut *tx)
        .await?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(row);
    }
    let target:Option<(Uuid,String,Uuid,Vec<u8>)>=sqlx::query_as(
        "SELECT r.revision_id,x.rule_key,r.id,r.input_hash FROM finance.check_result x JOIN finance.check_run r ON (r.tenant_id,r.company_id,r.id)=(x.tenant_id,x.company_id,x.run_id) JOIN finance.dossier d ON (d.tenant_id,d.company_id,d.id)=(r.tenant_id,r.company_id,r.dossier_id) JOIN finance.dossier_revision dr ON (dr.tenant_id,dr.company_id,dr.id)=(r.tenant_id,r.company_id,r.revision_id) JOIN finance.rule_version rv ON rv.tenant_id=r.tenant_id AND rv.company_id=r.company_id AND rv.policy_id=r.policy_id AND rv.rule_key=x.rule_key AND rv.state='PUBLISHED' WHERE x.tenant_id=$1 AND x.company_id=$2 AND x.id=$3 AND r.dossier_id=$4 AND d.owner_id=$5 AND d.version=$6 AND dr.number=d.revision AND r.dossier_version=d.version AND x.outcome='FAIL' AND rv.waivable",
    ).bind(t).bind(c).bind(result).bind(dossier).bind(a).bind(expected).fetch_optional(&mut *tx).await?;
    let Some((revision, rule_key, run, input_hash)) = target else {
        return Err(Error::Conflict);
    };
    if !verify_evidence(&mut tx, t, c, dossier, revision, &ev).await? {
        return Err(Error::InvalidInput);
    }
    sqlx::query("INSERT INTO finance.waiver(tenant_id,company_id,id,run_id,rule_key,dossier_id,revision_id,input_hash,requester_id,reason,expires_at,state,result_id,evidence_ids) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,clock_timestamp(),'REQUESTED',$11,$12)")
        .bind(t).bind(c).bind(id).bind(run).bind(&rule_key).bind(dossier).bind(revision).bind(input_hash).bind(a).bind(reason).bind(result).bind(&ev).execute(&mut *tx).await?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'WAIVER_REQUESTED')")
        .bind(t).bind(c).bind(command).bind(a).bind(correlation).bind(id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'WAIVER_REQUESTED')")
        .bind(t).bind(c).bind(command).bind(id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'waiver.request',$4,$5,$6)")
        .bind(t).bind(c).bind(a).bind(key.as_str()).bind(digest).bind(id).execute(&mut *tx).await?;
    let row = sqlx::query_as::<_, Waiver>(&format!(
        "{SELECT_WAIVER} WHERE w.tenant_id=$1 AND w.company_id=$2 AND w.id=$3"
    ))
    .bind(t)
    .bind(c)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
pub async fn decide_waiver(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    command: WaiverDecisionCommand<'_>,
) -> Result<Waiver, Error> {
    let WaiverDecisionCommand {
        dossier,
        waiver,
        decision,
        expires,
        reason,
        evidence,
    } = command;
    evidence_reason(reason, evidence)?;
    if !matches!(decision, "APPROVED" | "REJECTED") {
        return Err(Error::InvalidInput);
    }
    let Some(expected) = proof.context().expected_version else {
        return Err(Error::InvalidInput);
    };
    let expected = i64::from(expected);
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::FinanceApprover)
        .await
        .map_err(grant)?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/waiver.decide/{}", key.as_str()))
        .execute(&mut *tx)
        .await?;
    let mut ev = evidence.to_vec();
    ev.sort();
    ev.dedup();
    if ev.len() != evidence.len() {
        return Err(Error::InvalidInput);
    }
    let now = proof.context().occurred_at;
    let expiry = match (decision, expires) {
        ("APPROVED", Some(x)) if x > now && x <= now + chrono::Duration::days(365) => x,
        ("REJECTED", None) => now,
        _ => return Err(Error::InvalidInput),
    };
    let exp = expiry.to_rfc3339();
    let expected_bytes = expected.to_be_bytes();
    let mut ev_bytes = Vec::new();
    for e in &ev {
        ev_bytes.extend_from_slice(e.as_bytes())
    }
    let digest = hash(&[
        dossier.as_bytes(),
        waiver.as_bytes(),
        decision.as_bytes(),
        exp.as_bytes(),
        &expected_bytes,
        reason.as_bytes(),
        &ev_bytes,
    ]);
    let prior:Option<(Vec<u8>,Uuid)>=sqlx::query_as("SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='waiver.decide' AND key=$4")
        .bind(t).bind(c).bind(a).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((old, wid)) = prior {
        if old != digest || wid != waiver {
            return Err(Error::Conflict);
        }
        let row = sqlx::query_as::<_, Waiver>(&format!(
            "{SELECT_WAIVER} WHERE w.tenant_id=$1 AND w.company_id=$2 AND w.id=$3"
        ))
        .bind(t)
        .bind(c)
        .bind(waiver)
        .fetch_one(&mut *tx)
        .await?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(row);
    }
    let binding:Option<(Uuid,Uuid)>=sqlx::query_as("SELECT w.revision_id,w.requester_id FROM finance.waiver w JOIN finance.dossier d ON (d.tenant_id,d.company_id,d.id)=(w.tenant_id,w.company_id,w.dossier_id) WHERE w.tenant_id=$1 AND w.company_id=$2 AND w.id=$3 AND w.dossier_id=$4 AND w.state='REQUESTED' AND d.version=$5 FOR UPDATE OF w")
        .bind(t).bind(c).bind(waiver).bind(dossier).bind(expected).fetch_optional(&mut *tx).await?;
    let Some((revision, requester)) = binding else {
        return Err(Error::Conflict);
    };
    if requester == a {
        return Err(Error::Forbidden);
    }
    if !verify_evidence(&mut tx, t, c, dossier, revision, &ev).await? {
        return Err(Error::InvalidInput);
    }
    sqlx::query("UPDATE finance.waiver SET state=$4,approver_id=$5,expires_at=$6::text::timestamptz,decision_reason=$7,decision_evidence_ids=$8 WHERE tenant_id=$1 AND company_id=$2 AND id=$3")
        .bind(t).bind(c).bind(waiver).bind(decision).bind(a).bind(&exp).bind(reason).bind(&ev).execute(&mut *tx).await?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'WAIVER_DECIDED')")
        .bind(t).bind(c).bind(command).bind(a).bind(correlation).bind(waiver).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'WAIVER_DECIDED')")
        .bind(t).bind(c).bind(command).bind(waiver).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'waiver.decide',$4,$5,$6)")
        .bind(t).bind(c).bind(a).bind(key.as_str()).bind(digest).bind(waiver).execute(&mut *tx).await?;
    let row = sqlx::query_as::<_, Waiver>(&format!(
        "{SELECT_WAIVER} WHERE w.tenant_id=$1 AND w.company_id=$2 AND w.id=$3"
    ))
    .bind(t)
    .bind(c)
    .bind(waiver)
    .fetch_one(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}
