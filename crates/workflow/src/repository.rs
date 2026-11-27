//! E18 immutable, policy-bound workflow registry.
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, lock_roles, require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
    ids::{CompanyScope, ResourceScope},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub command: String,
    pub from: String,
    pub to: String,
    pub role: String,
    pub guard_refs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, sqlx::FromRow)]
pub struct Workflow {
    pub id: Uuid,
    pub policy_id: Uuid,
    pub version: i64,
    pub name: String,
    pub state: String,
    pub hash: String,
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

fn grant(error: GrantError) -> Error {
    match error {
        GrantError::Forbidden => Error::Forbidden,
        GrantError::Database(error) => Error::Database(error),
    }
}

fn request_hash(parts: &[&[u8]]) -> Vec<u8> {
    let mut digest = Sha256::new();
    digest.update(b"finance.workflow.command/v1");
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    digest.finalize().to_vec()
}

fn required_guards(command: &str, from: &str, to: &str, role: &str) -> Option<&'static [&'static str]> {
    match (command, from, to, role) {
        ("PROCESS","DRAFT","PROCESSING","REQUESTER") | ("PROCESS","NEEDS_INPUT","PROCESSING","REQUESTER")
            => Some(&["authorized","current_revision","clean_docs"]),
        ("PROCESS_COMPLETE","PROCESSING","REVIEWABLE","SYSTEM")
            => Some(&["authorized","current_revision","worker_fenced","complete"]),
        ("PROCESS_COMPLETE","PROCESSING","NEEDS_INPUT","SYSTEM")
            => Some(&["authorized","current_revision","worker_fenced","incomplete"]),
        ("READY","REVIEWABLE","READY","REQUESTER")
            => Some(&["authorized","current_revision","complete","verified","rules_pass","reserved","snapshot_current"]),
        ("EXPORT","READY","EXPORTED","SYSTEM")
            => Some(&["authorized","current_revision","artifact_verified","snapshot_current"]),
        ("HANDOFF","EXPORTED","HANDED_OFF","REQUESTER")
            => Some(&["authorized","current_revision","external_receipt","reserved","snapshot_current","rules_pass"]),
        ("BEGIN_REVIEW","HANDED_OFF","ACCOUNTING_REVIEW","REQUESTER")
            => Some(&["authorized","current_revision","independent_accountant"]),
        ("ACCEPT","ACCOUNTING_REVIEW","ACCEPTED","ACCOUNTANT")
            => Some(&["authorized","current_revision","independent_accountant","critical_confirmed","required_external_evidence","snapshot_current","rules_pass"]),
        ("RETURN","ACCOUNTING_REVIEW","RETURNED","ACCOUNTANT")
            => Some(&["authorized","current_revision","independent_accountant","return_reason","reconciled"]),
        ("REOPEN","RETURNED","DRAFT","REQUESTER")
            => Some(&["authorized","current_revision","reconciled"]),
        _ => None,
    }
}

fn expected_edges() -> BTreeSet<(String,String,String,String)> {
    [
        ("PROCESS","DRAFT","PROCESSING","REQUESTER"),
        ("PROCESS","NEEDS_INPUT","PROCESSING","REQUESTER"),
        ("PROCESS_COMPLETE","PROCESSING","REVIEWABLE","SYSTEM"),
        ("PROCESS_COMPLETE","PROCESSING","NEEDS_INPUT","SYSTEM"),
        ("READY","REVIEWABLE","READY","REQUESTER"),
        ("EXPORT","READY","EXPORTED","SYSTEM"),
        ("HANDOFF","EXPORTED","HANDED_OFF","REQUESTER"),
        ("BEGIN_REVIEW","HANDED_OFF","ACCOUNTING_REVIEW","REQUESTER"),
        ("ACCEPT","ACCOUNTING_REVIEW","ACCEPTED","ACCOUNTANT"),
        ("RETURN","ACCOUNTING_REVIEW","RETURNED","ACCOUNTANT"),
        ("REOPEN","RETURNED","DRAFT","REQUESTER"),
    ].into_iter().map(|(a,b,c,d)|(a.into(),b.into(),c.into(),d.into())).collect()
}

pub fn compile_definition(transitions: &[Transition], version: i64) -> Result<String, Error> {
    if transitions.len()!=11 || !(1..=9_007_199_254_740_991).contains(&version) {
        return Err(Error::InvalidInput);
    }
    let allowed: BTreeSet<&str> = [
        "authorized","current_revision","clean_docs","worker_fenced","complete","incomplete",
        "verified","rules_pass","reserved","snapshot_current","artifact_verified","external_receipt",
        "independent_accountant","critical_confirmed","required_external_evidence","return_reason","reconciled",
    ].into_iter().collect();
    let mut normalized=transitions.to_vec();
    let mut seen=BTreeSet::new();
    for item in &mut normalized {
        if item.command.len()>64 || item.from.len()>64 || item.to.len()>64 || item.role.len()>64
            || item.guard_refs.is_empty() || item.guard_refs.len()>20
            || [&item.command,&item.from,&item.to,&item.role].into_iter()
                .any(|value| value.trim()!=value || value.chars().any(char::is_control))
        { return Err(Error::InvalidInput); }
        let original_len=item.guard_refs.len();
        item.guard_refs.sort();
        item.guard_refs.dedup();
        if item.guard_refs.len()!=original_len { return Err(Error::InvalidInput); }
        let Some(required)=required_guards(&item.command,&item.from,&item.to,&item.role) else {
            return Err(Error::InvalidInput);
        };
        let guards:BTreeSet<&str>=item.guard_refs.iter().map(String::as_str).collect();
        if guards.iter().any(|g|!allowed.contains(g)) || required.iter().any(|g|!guards.contains(g)) {
            return Err(Error::InvalidInput);
        }
        if !seen.insert((item.command.clone(),item.from.clone(),item.to.clone(),item.role.clone())) {
            return Err(Error::InvalidInput);
        }
    }
    if seen!=expected_edges() { return Err(Error::InvalidInput); }
    normalized.sort();
    serde_json::to_string(&json!({"version":version,"transitions":normalized}))
        .map_err(|_|Error::InvalidInput)
}

const SELECT: &str = "SELECT workflow_id AS id,policy_id,version,name,CASE WHEN state='REVOKED' THEN 'RETIRED' ELSE state END AS state,encode(sha256(convert_to(definition::text,'UTF8')),'hex') AS hash FROM finance.workflow_version";

async fn allowed_read(tx:&mut Transaction<'_,Postgres>,proof:&AuthenticatedCommand)->Result<(),Error>{
    let roles=lock_roles(tx,proof.context()).await.map_err(grant)?;
    if roles.contains(&Role::Requester)||roles.contains(&Role::PolicyAdmin){Ok(())}else{Err(Error::Forbidden)}
}
async fn latest(tx:&mut Transaction<'_,Postgres>,t:Uuid,c:Uuid,w:Uuid)->Result<Workflow,Error>{
    Ok(sqlx::query_as::<_,Workflow>(&format!(
        "{SELECT} WHERE tenant_id=$1 AND company_id=$2 AND workflow_id=$3 ORDER BY version DESC LIMIT 1"
    )).bind(t).bind(c).bind(w).fetch_one(&mut **tx).await?)
}

pub struct CreateDraft<'a>{pub policy:Uuid,pub workflow:Uuid,pub row:Uuid,pub name:&'a str,pub transitions:&'a [Transition]}
pub async fn create(pool:&sqlx::PgPool,proof:&AuthenticatedCommand,key:&IdempotencyKey,d:CreateDraft<'_>)->Result<Workflow,Error>{
    if d.policy.is_nil()||d.workflow.is_nil()||d.row.is_nil()||d.workflow==d.row||d.name.trim().is_empty()
        ||d.name.len()>200||d.name.chars().any(char::is_control)||proof.context().expected_version.is_some()
    {return Err(Error::InvalidInput);}
    let definition=compile_definition(d.transitions,1)?;
    let hash=request_hash(&[d.policy.as_bytes(),d.name.as_bytes(),definition.as_bytes()]);
    let (t,c,a)=binding(proof)?;
    let mut tx=scoped_transaction(pool,proof.context()).await?;
    require_role(&mut tx,proof.context(),Role::PolicyAdmin).await.map_err(grant)?;
    recheck_authenticated(&mut tx,Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/workflow.create/{}",key.as_str())).execute(&mut *tx).await?;
    let prior:Option<(Vec<u8>,Uuid)>=sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='workflow.create' AND key=$4"
    ).bind(t).bind(c).bind(a).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((old,result))=prior{
        if old!=hash{return Err(Error::Conflict);}
        let receipt=latest(&mut tx,t,c,result).await?;
        recheck_authenticated(&mut tx,Some(proof)).await?;tx.commit().await?;return Ok(receipt);
    }
    let policy_ok:bool=sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM finance.policy_version WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND author_id=$4 AND state='DRAFT')"
    ).bind(t).bind(c).bind(d.policy).bind(a).fetch_one(&mut *tx).await?;
    if !policy_ok{return Err(Error::Conflict);}
    sqlx::query("INSERT INTO finance.workflow_version(tenant_id,company_id,id,workflow_id,policy_id,name,version,state,definition,author_id) VALUES($1,$2,$3,$4,$5,$6,1,'DRAFT',$7::text::jsonb,$8)")
        .bind(t).bind(c).bind(d.row).bind(d.workflow).bind(d.policy).bind(d.name).bind(&definition).bind(a).execute(&mut *tx).await?;
    let command:Uuid=proof.context().command.into();let correlation:Uuid=proof.context().correlation.into();
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'WORKFLOW_DRAFT_CREATED')")
        .bind(t).bind(c).bind(command).bind(a).bind(correlation).bind(d.workflow).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'WORKFLOW_DRAFT_CREATED')")
        .bind(t).bind(c).bind(command).bind(d.workflow).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'workflow.create',$4,$5,$6)")
        .bind(t).bind(c).bind(a).bind(key.as_str()).bind(hash).bind(d.workflow).execute(&mut *tx).await?;
    let receipt=latest(&mut tx,t,c,d.workflow).await?;
    recheck_authenticated(&mut tx,Some(proof)).await?;tx.commit().await?;Ok(receipt)
}

pub struct ReviseDraft<'a>{pub workflow:Uuid,pub row:Uuid,pub name:&'a str,pub transitions:&'a [Transition],pub reason:&'a str}
pub async fn revise(pool:&sqlx::PgPool,proof:&AuthenticatedCommand,key:&IdempotencyKey,d:ReviseDraft<'_>)->Result<Workflow,Error>{
    let Some(expected)=proof.context().expected_version else{return Err(Error::InvalidInput)};let expected=i64::from(expected);
    if d.workflow.is_nil()||d.row.is_nil()||d.name.trim().is_empty()||d.name.len()>200
        ||d.reason.trim().is_empty()||d.reason.len()>2048||d.reason.chars().any(char::is_control)
    {return Err(Error::InvalidInput);}
    let next=expected.checked_add(1).ok_or(Error::Conflict)?;
    let definition=compile_definition(d.transitions,next)?;
    let hash=request_hash(&[d.workflow.as_bytes(),&expected.to_be_bytes(),d.name.as_bytes(),definition.as_bytes(),d.reason.as_bytes()]);
    let (t,c,a)=binding(proof)?;let mut tx=scoped_transaction(pool,proof.context()).await?;
    require_role(&mut tx,proof.context(),Role::PolicyAdmin).await.map_err(grant)?;
    recheck_authenticated(&mut tx,Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/workflow.revise/{}",key.as_str())).execute(&mut *tx).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("finance.workflow/v1/{t}/{c}/{}",d.workflow)).execute(&mut *tx).await?;
    let prior:Option<(Vec<u8>,Uuid)>=sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='workflow.revise' AND key=$4"
    ).bind(t).bind(c).bind(a).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((old,result))=prior{
        if old!=hash||result!=d.workflow{return Err(Error::Conflict);}
        let receipt=latest(&mut tx,t,c,d.workflow).await?;
        recheck_authenticated(&mut tx,Some(proof)).await?;tx.commit().await?;return Ok(receipt);
    }
    let current=latest(&mut tx,t,c,d.workflow).await?;
    if current.version!=expected||current.state!="DRAFT"{return Err(Error::Conflict);}
    let affected=sqlx::query(
        "INSERT INTO finance.workflow_version(tenant_id,company_id,id,workflow_id,policy_id,name,version,state,definition,author_id) SELECT tenant_id,company_id,$4,workflow_id,policy_id,$5,$6,'DRAFT',$7::text::jsonb,author_id FROM finance.workflow_version WHERE tenant_id=$1 AND company_id=$2 AND workflow_id=$3 AND version=$8 AND author_id=$9"
    ).bind(t).bind(c).bind(d.workflow).bind(d.row).bind(d.name).bind(next).bind(&definition).bind(expected).bind(a)
     .execute(&mut *tx).await?.rows_affected();
    if affected!=1{return Err(Error::Conflict);}
    let command:Uuid=proof.context().command.into();let correlation:Uuid=proof.context().correlation.into();
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'WORKFLOW_DRAFT_REVISED')")
        .bind(t).bind(c).bind(command).bind(a).bind(correlation).bind(d.workflow).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'WORKFLOW_DRAFT_REVISED')")
        .bind(t).bind(c).bind(command).bind(d.workflow).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'workflow.revise',$4,$5,$6)")
        .bind(t).bind(c).bind(a).bind(key.as_str()).bind(hash).bind(d.workflow).execute(&mut *tx).await?;
    let receipt=latest(&mut tx,t,c,d.workflow).await?;
    recheck_authenticated(&mut tx,Some(proof)).await?;tx.commit().await?;Ok(receipt)
}

pub async fn list(pool:&sqlx::PgPool,proof:&AuthenticatedCommand,after:Uuid,limit:i64)->Result<Vec<Workflow>,Error>{
    if !(1..=100).contains(&limit){return Err(Error::InvalidInput);}
    let(t,c,_)=binding(proof)?;let mut tx=scoped_transaction(pool,proof.context()).await?;
    allowed_read(&mut tx,proof).await?;recheck_authenticated(&mut tx,Some(proof)).await?;
    let rows=sqlx::query_as::<_,Workflow>(
        "SELECT id,policy_id,version,name,state,hash FROM (SELECT DISTINCT ON(workflow_id) workflow_id AS id,policy_id,version,name,CASE WHEN state='REVOKED' THEN 'RETIRED' ELSE state END AS state,encode(sha256(convert_to(definition::text,'UTF8')),'hex') AS hash FROM finance.workflow_version WHERE tenant_id=$1 AND company_id=$2 AND workflow_id>$3 ORDER BY workflow_id,version DESC) w ORDER BY id LIMIT $4"
    ).bind(t).bind(c).bind(after).bind(limit).fetch_all(&mut *tx).await?;
    recheck_authenticated(&mut tx,Some(proof)).await?;tx.commit().await?;Ok(rows)
}
pub async fn get(pool:&sqlx::PgPool,proof:&AuthenticatedCommand,workflow:Uuid)->Result<Option<Workflow>,Error>{
    if workflow.is_nil(){return Err(Error::InvalidInput);}
    let(t,c,_)=binding(proof)?;let mut tx=scoped_transaction(pool,proof.context()).await?;
    allowed_read(&mut tx,proof).await?;recheck_authenticated(&mut tx,Some(proof)).await?;
    let row=sqlx::query_as::<_,Workflow>(&format!(
        "{SELECT} WHERE tenant_id=$1 AND company_id=$2 AND workflow_id=$3 ORDER BY version DESC LIMIT 1"
    )).bind(t).bind(c).bind(workflow).fetch_optional(&mut *tx).await?;
    recheck_authenticated(&mut tx,Some(proof)).await?;tx.commit().await?;Ok(row)
}
