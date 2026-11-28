//! E15 authenticated immutable rule draft repository.
use crate::compiler::{self, Severity, Type};
use finance_core::prototype::{Error, recheck_authenticated, scoped_transaction};
use finance_identity::AuthenticatedCommand;
use finance_permissions::{GrantError, Role, lock_roles, require_role};
use finance_shared::{
    contracts::Principal,
    http_contract::IdempotencyKey,
    ids::{CompanyScope, ResourceScope},
    money::Currency,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, sqlx::FromRow)]
pub struct Rule {
    pub id: Uuid,
    pub policy_id: Uuid,
    pub version: i64,
    pub name: String,
    pub severity: String,
    pub waivable: bool,
    pub clause_id: Uuid,
    pub definition_json: String,
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

fn registry() -> BTreeMap<String, Type> {
    let vnd = Currency::parse("VND").expect("VND is a valid ISO currency code");
    [
        ("invoice.total", Type::Money(vnd)),
        ("invoice.net", Type::Money(vnd)),
        ("invoice.tax", Type::Money(vnd)),
        ("contract.remaining", Type::Money(vnd)),
        ("budget.remaining", Type::Money(vnd)),
        ("advance.remaining", Type::Money(vnd)),
        ("invoice.quantity", Type::Decimal),
        ("invoice.date", Type::Date),
        ("invoice.verified", Type::Boolean),
        ("vendor.verified", Type::Boolean),
        ("reference.fresh", Type::Boolean),
    ]
    .into_iter()
    .map(|(path, ty)| (path.to_owned(), ty))
    .collect()
}

fn validate_definition(
    name: &str,
    definition: &Value,
    version: u64,
) -> Result<(String, bool, Uuid), Error> {
    if name.is_empty()
        || name.len() > 64
        || !name.as_bytes()[0].is_ascii_alphabetic()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(Error::InvalidInput);
    }
    let bytes = serde_json::to_vec(definition).map_err(|_| Error::InvalidInput)?;
    let compiled = compiler::compile(&bytes, &registry()).map_err(|_| Error::InvalidInput)?;
    if compiled.rule_id() != name || compiled.version() != version {
        return Err(Error::InvalidInput);
    }
    let severity = match compiled.severity() {
        Severity::Block => "BLOCK",
        Severity::Warning => "WARNING",
        Severity::Info => "INFO",
    };
    Ok((
        severity.to_owned(),
        compiled.waivable(),
        compiled.clause_id(),
    ))
}

fn request_hash(policy: Uuid, name: &str, definition: &Value) -> Result<Vec<u8>, Error> {
    let payload = serde_json::to_vec(definition).map_err(|_| Error::InvalidInput)?;
    let mut hash = Sha256::new();
    hash.update(b"finance.rule.create/v1");
    hash.update(policy.as_bytes());
    hash.update((name.len() as u64).to_be_bytes());
    hash.update(name.as_bytes());
    hash.update((payload.len() as u64).to_be_bytes());
    hash.update(payload);
    Ok(hash.finalize().to_vec())
}

const SELECT_RULE: &str = "SELECT rule_id AS id,policy_id,version,rule_key AS name,severity,waivable,clause_id,definition::text AS definition_json FROM finance.rule_version";

async fn allowed_read(
    tx: &mut Transaction<'_, Postgres>,
    proof: &AuthenticatedCommand,
) -> Result<(), Error> {
    let roles = lock_roles(tx, proof.context()).await.map_err(|e| match e {
        GrantError::Forbidden => Error::Forbidden,
        GrantError::Database(e) => Error::Database(e),
    })?;
    if roles.contains(&Role::Requester) || roles.contains(&Role::PolicyAdmin) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

pub async fn create(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    policy: Uuid,
    id: Uuid,
    name: &str,
    definition: &Value,
) -> Result<Rule, Error> {
    if policy.is_nil() || id.is_nil() || proof.context().expected_version.is_some() {
        return Err(Error::InvalidInput);
    }
    let (severity, waivable, clause) = validate_definition(name, definition, 1)?;
    let payload = serde_json::to_string(definition).map_err(|_| Error::InvalidInput)?;
    let hash = request_hash(policy, name, definition)?;
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::PolicyAdmin)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/rule.create/{}", key.as_str()))
        .execute(&mut *tx)
        .await?;
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='rule.create' AND key=$4",
    )
    .bind(t).bind(c).bind(a).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((old, result)) = prior {
        if old != hash {
            return Err(Error::Conflict);
        }
        let row = sqlx::query_as::<_, Rule>(&format!(
            "{SELECT_RULE} WHERE tenant_id=$1 AND company_id=$2 AND id=$3"
        ))
        .bind(t)
        .bind(c)
        .bind(result)
        .fetch_one(&mut *tx)
        .await?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(row);
    }
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM finance.policy_version p JOIN finance.policy_clause cl ON (cl.tenant_id,cl.company_id,cl.policy_id)=(p.tenant_id,p.company_id,p.id) WHERE p.tenant_id=$1 AND p.company_id=$2 AND p.id=$3 AND p.author_id=$4 AND p.state='DRAFT' AND cl.id=$5)",
    ).bind(t).bind(c).bind(policy).bind(a).bind(clause).fetch_one(&mut *tx).await?;
    if !allowed {
        return Err(Error::Conflict);
    }
    sqlx::query(
        "INSERT INTO finance.rule_version(tenant_id,company_id,id,policy_id,clause_id,rule_key,version,severity,waivable,state,definition,author_id) VALUES($1,$2,$3,$4,$5,$6,1,$7,$8,'DRAFT',$9::text::jsonb,$10)",
    )
    .bind(t).bind(c).bind(id).bind(policy).bind(clause).bind(name).bind(&severity).bind(waivable).bind(&payload).bind(a)
    .execute(&mut *tx).await?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'RULE_DRAFT_CREATED')")
        .bind(t).bind(c).bind(command).bind(a).bind(correlation).bind(id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'RULE_DRAFT_CREATED')")
        .bind(t).bind(c).bind(command).bind(id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id) VALUES($1,$2,$3,'rule.create',$4,$5,$6)")
        .bind(t).bind(c).bind(a).bind(key.as_str()).bind(hash).bind(id).execute(&mut *tx).await?;
    let row = sqlx::query_as::<_, Rule>(&format!(
        "{SELECT_RULE} WHERE tenant_id=$1 AND company_id=$2 AND id=$3"
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

pub async fn list(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    after: Uuid,
    limit: i64,
) -> Result<Vec<Rule>, Error> {
    if !(1..=100).contains(&limit) {
        return Err(Error::InvalidInput);
    }
    let (t, c, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    allowed_read(&mut tx, proof).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let rows = sqlx::query_as::<_, Rule>(
        "SELECT * FROM (SELECT DISTINCT ON(rule_id) rule_id AS id,policy_id,version,rule_key AS name,severity,waivable,clause_id,definition::text AS definition_json FROM finance.rule_version WHERE tenant_id=$1 AND company_id=$2 AND rule_id>$3 ORDER BY rule_id,version DESC) rows ORDER BY id LIMIT $4",
    )
    .bind(t)
    .bind(c)
    .bind(after)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(rows)
}

pub async fn get(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    id: Uuid,
) -> Result<Option<Rule>, Error> {
    if id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (t, c, _) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    allowed_read(&mut tx, proof).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let row = sqlx::query_as::<_, Rule>(&format!(
        "{SELECT_RULE} WHERE tenant_id=$1 AND company_id=$2 AND rule_id=$3 ORDER BY version DESC LIMIT 1"
    ))
    .bind(t)
    .bind(c)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(row)
}

pub struct ReviseDraft<'a> {
    pub rule: Uuid,
    pub row: Uuid,
    pub name: &'a str,
    pub definition: &'a Value,
    pub reason: &'a str,
}

pub async fn revise(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    draft: ReviseDraft<'_>,
) -> Result<Rule, Error> {
    let expected = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(Error::InvalidInput)?,
    );
    if draft.rule.is_nil()
        || draft.row.is_nil()
        || !(1..9_007_199_254_740_991).contains(&expected)
        || draft.reason.trim().is_empty()
        || draft.reason.len() > 2048
        || draft.reason.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput);
    }
    let next = expected + 1;
    let (severity, waivable, clause) =
        validate_definition(draft.name, draft.definition, next as u64)?;
    let payload = serde_json::to_string(draft.definition).map_err(|_| Error::InvalidInput)?;
    let mut hash = Sha256::new();
    hash.update(b"finance.rule.revise/v1");
    hash.update(draft.rule.as_bytes());
    hash.update(expected.to_be_bytes());
    for value in [draft.name, payload.as_str(), draft.reason] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    let hash = hash.finalize().to_vec();
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::PolicyAdmin)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    for lock in [
        format!("{t}/{c}/{a}/rule.revise/{}", key.as_str()),
        format!("finance.rule/v1/{t}/{c}/{}", draft.rule),
    ] {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(lock)
            .execute(&mut *tx)
            .await?;
    }
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='rule.revise' AND key=$4",
    ).bind(t).bind(c).bind(a).bind(key.as_str()).fetch_optional(&mut *tx).await?;
    if let Some((old, result)) = prior {
        if old != hash {
            return Err(Error::Conflict);
        }
        let receipt = sqlx::query_as::<_, Rule>(&format!(
            "{SELECT_RULE} WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND rule_id=$4 AND author_id=$5"
        )).bind(t).bind(c).bind(result).bind(draft.rule).bind(a)
          .fetch_optional(&mut *tx).await?.ok_or(Error::Forbidden)?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok(receipt);
    }
    let current: Option<(Uuid, i64, String, String)> = sqlx::query_as(
        "SELECT policy_id,version,rule_key,state FROM finance.rule_version WHERE tenant_id=$1 AND company_id=$2 AND rule_id=$3 AND author_id=$4 ORDER BY version DESC LIMIT 1",
    ).bind(t).bind(c).bind(draft.rule).bind(a).fetch_optional(&mut *tx).await?;
    let Some((policy, version, name, state)) = current else {
        return Err(Error::Forbidden);
    };
    if version != expected || state != "DRAFT" || name != draft.name {
        return Err(Error::Conflict);
    }
    let allowed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM finance.policy_version p JOIN finance.policy_clause cl ON (cl.tenant_id,cl.company_id,cl.policy_id)=(p.tenant_id,p.company_id,p.id) WHERE p.tenant_id=$1 AND p.company_id=$2 AND p.id=$3 AND p.author_id=$4 AND p.state='DRAFT' AND cl.id=$5)",
    ).bind(t).bind(c).bind(policy).bind(a).bind(clause).fetch_one(&mut *tx).await?;
    if !allowed {
        return Err(Error::Conflict);
    }
    sqlx::query("INSERT INTO finance.rule_version(tenant_id,company_id,id,rule_id,policy_id,clause_id,rule_key,version,severity,waivable,state,definition,author_id,revision_reason) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'DRAFT',$11::text::jsonb,$12,$13)")
        .bind(t).bind(c).bind(draft.row).bind(draft.rule).bind(policy).bind(clause)
        .bind(draft.name).bind(next).bind(&severity).bind(waivable).bind(&payload).bind(a).bind(draft.reason)
        .execute(&mut *tx).await?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'RULE_DRAFT_REVISED')")
        .bind(t).bind(c).bind(command).bind(a).bind(correlation).bind(draft.rule).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'RULE_DRAFT_REVISED')")
        .bind(t).bind(c).bind(command).bind(draft.rule).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_version) VALUES($1,$2,$3,'rule.revise',$4,$5,$6,$7)")
        .bind(t).bind(c).bind(a).bind(key.as_str()).bind(hash).bind(draft.row).bind(next)
        .execute(&mut *tx).await?;
    let receipt = sqlx::query_as::<_, Rule>(&format!(
        "{SELECT_RULE} WHERE tenant_id=$1 AND company_id=$2 AND id=$3"
    ))
    .bind(t)
    .bind(c)
    .bind(draft.row)
    .fetch_one(&mut *tx)
    .await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(receipt)
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Simulation {
    pub id: Uuid,
    pub rule_version_id: Uuid,
    pub corpus_id: Uuid,
    pub pass_count: u32,
    pub fail_count: u32,
    pub unknown_count: u32,
    pub error_count: u32,
    pub not_applicable_count: u32,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CorpusFact {
    value: Option<Value>,
    verified: bool,
    fresh: bool,
    evidence: Vec<finance_shared::ids::EvidenceId>,
}

fn corpus_facts(json: &str) -> Result<Vec<BTreeMap<String, compiler::Fact>>, Error> {
    use finance_shared::money::{Amount, Money};
    if json.len() > 4 * 1024 * 1024 {
        return Err(Error::InvalidInput);
    }
    let rows: Vec<BTreeMap<String, CorpusFact>> =
        serde_json::from_str(json).map_err(|_| Error::InvalidInput)?;
    if !(1..=5000).contains(&rows.len()) {
        return Err(Error::InvalidInput);
    }
    let types = registry();
    rows.into_iter()
        .map(|row| {
            if row.len() > types.len() {
                return Err(Error::InvalidInput);
            }
            row.into_iter()
                .map(|(path, raw)| {
                    let ty = types.get(&path).ok_or(Error::InvalidInput)?;
                    if raw.evidence.len() > 64 {
                        return Err(Error::InvalidInput);
                    }
                    let value = match (ty, raw.value) {
                        (_, None | Some(Value::Null)) => None,
                        (Type::Boolean, Some(Value::Bool(b))) => Some(compiler::Value::Boolean(b)),
                        (Type::Date, Some(Value::String(s))) => Some(compiler::Value::Date(
                            compiler::parse_date(&s).map_err(|_| Error::InvalidInput)?,
                        )),
                        (Type::Decimal, Some(Value::String(s))) => Some(compiler::Value::Decimal(
                            Amount::parse(&s).map_err(|_| Error::InvalidInput)?,
                        )),
                        (Type::Money(currency), Some(Value::String(s))) => {
                            Some(compiler::Value::Money(Money {
                                amount: Amount::parse(&s).map_err(|_| Error::InvalidInput)?,
                                currency: *currency,
                            }))
                        }
                        _ => return Err(Error::InvalidInput),
                    };
                    Ok((
                        path,
                        compiler::Fact {
                            value,
                            verified: raw.verified,
                            fresh: raw.fresh,
                            evidence: raw.evidence,
                        },
                    ))
                })
                .collect()
        })
        .collect()
}

pub struct SimulationInput {
    pub rule: Uuid,
    pub corpus: Uuid,
    pub id: Uuid,
}

pub async fn simulate(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    key: &IdempotencyKey,
    input: SimulationInput,
) -> Result<(Simulation, i64), Error> {
    let expected = i64::from(
        proof
            .context()
            .expected_version
            .ok_or(Error::InvalidInput)?,
    );
    if input.rule.is_nil() || input.corpus.is_nil() || input.id.is_nil() || expected < 1 {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::PolicyAdmin)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("{t}/{c}/{a}/rule.simulate/{}", key.as_str()))
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(format!("finance.rule/v1/{t}/{c}/{}", input.rule))
        .execute(&mut *tx)
        .await?;
    let corpus: Option<(String, Vec<u8>)> = sqlx::query_as(
        "SELECT facts::text,corpus_digest FROM finance.simulation_corpus WHERE tenant_id=$1 AND company_id=$2 AND id=$3",
    )
    .bind(t)
    .bind(c)
    .bind(input.corpus)
    .fetch_optional(&mut *tx)
    .await?;
    let (json, digest) = corpus.ok_or(Error::Forbidden)?;
    let mut h = Sha256::new();
    h.update(b"finance.rule.simulate/v1");
    h.update(input.rule.as_bytes());
    h.update(expected.to_be_bytes());
    h.update(input.corpus.as_bytes());
    h.update(&digest);
    let request_hash = h.finalize().to_vec();
    let prior: Option<(Vec<u8>, Uuid)> = sqlx::query_as(
        "SELECT request_hash,result_id FROM finance.idempotency_record WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND operation_id='rule.simulate' AND key=$4",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(key.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((old, id)) = prior {
        if old != request_hash {
            return Err(Error::Conflict);
        }
        let payload: String = sqlx::query_scalar(
            "SELECT result::text FROM finance.rule_simulation WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND actor_id=$4",
        )
        .bind(t)
        .bind(c)
        .bind(id)
        .bind(a)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(Error::Forbidden)?;
        let result = serde_json::from_str(&payload).map_err(|_| Error::Conflict)?;
        recheck_authenticated(&mut tx, Some(proof)).await?;
        tx.commit().await?;
        return Ok((result, expected));
    }
    let current: Option<(Uuid, i64, String)> = sqlx::query_as(
        "SELECT id,version,definition::text FROM finance.rule_version WHERE tenant_id=$1 AND company_id=$2 AND rule_id=$3 AND author_id=$4 ORDER BY version DESC LIMIT 1",
    )
    .bind(t)
    .bind(c)
    .bind(input.rule)
    .bind(a)
    .fetch_optional(&mut *tx)
    .await?;
    let (row, version, definition) = current.ok_or(Error::Forbidden)?;
    if version != expected {
        return Err(Error::Conflict);
    }
    let compiled =
        compiler::compile(definition.as_bytes(), &registry()).map_err(|_| Error::InvalidInput)?;
    let facts = corpus_facts(&json)?;
    let mut result = Simulation {
        id: input.id,
        rule_version_id: row,
        corpus_id: input.corpus,
        pass_count: 0,
        fail_count: 0,
        unknown_count: 0,
        error_count: 0,
        not_applicable_count: 0,
    };
    for fact in &facts {
        match compiler::evaluate(&compiled, fact).outcome {
            crate::Outcome::Pass => result.pass_count += 1,
            crate::Outcome::Fail => result.fail_count += 1,
            crate::Outcome::Unknown => result.unknown_count += 1,
            crate::Outcome::Error => result.error_count += 1,
            crate::Outcome::NotApplicable => result.not_applicable_count += 1,
        }
    }
    let baseline: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT id,definition::text FROM finance.rule_version WHERE tenant_id=$1 AND company_id=$2 AND rule_id=$3 AND version<$4 ORDER BY version DESC LIMIT 1",
    )
    .bind(t)
    .bind(c)
    .bind(input.rule)
    .bind(version)
    .fetch_optional(&mut *tx)
    .await?;
    let comparison = if let Some((baseline_id, definition)) = baseline {
        let old = compiler::compile(definition.as_bytes(), &registry())
            .map_err(|_| Error::InvalidInput)?;
        let comparison =
            crate::simulator::compare(&old, &compiled, &facts).map_err(|_| Error::InvalidInput)?;
        Some(
            serde_json::json!({
                "baseline_rule_version_id": baseline_id,
                "transitions": comparison.transitions,
                "evaluated": comparison.evaluated
            })
            .to_string(),
        )
    } else {
        None
    };
    let payload = serde_json::to_string(&result).map_err(|_| Error::InvalidInput)?;
    let command: Uuid = proof.context().command.into();
    let correlation: Uuid = proof.context().correlation.into();
    sqlx::query("INSERT INTO finance.rule_simulation(tenant_id,company_id,id,rule_id,actor_id,corpus_id,corpus_digest,result,comparison) VALUES($1,$2,$3,$4,$5,$6,$7,$8::text::jsonb,$9::text::jsonb)")
        .bind(t).bind(c).bind(result.id).bind(row).bind(a).bind(input.corpus).bind(digest)
        .bind(payload).bind(comparison).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.audit_event(tenant_id,company_id,id,actor_id,command_id,correlation_id,resource_id,action) VALUES($1,$2,$3,$4,$3,$5,$6,'RULE_SIMULATED')")
        .bind(t).bind(c).bind(command).bind(a).bind(correlation).bind(result.id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'RULE_SIMULATED')")
        .bind(t).bind(c).bind(command).bind(result.id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO finance.idempotency_record(tenant_id,company_id,actor_id,operation_id,key,request_hash,result_id,result_version) VALUES($1,$2,$3,'rule.simulate',$4,$5,$6,$7)")
        .bind(t).bind(c).bind(a).bind(key.as_str()).bind(request_hash).bind(result.id).bind(version).execute(&mut *tx).await?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok((result, version))
}

pub async fn get_simulation(
    pool: &sqlx::PgPool,
    proof: &AuthenticatedCommand,
    id: Uuid,
) -> Result<Option<Simulation>, Error> {
    if id.is_nil() {
        return Err(Error::InvalidInput);
    }
    let (t, c, a) = binding(proof)?;
    let mut tx = scoped_transaction(pool, proof.context()).await?;
    require_role(&mut tx, proof.context(), Role::PolicyAdmin)
        .await
        .map_err(|e| match e {
            GrantError::Forbidden => Error::Forbidden,
            GrantError::Database(e) => Error::Database(e),
        })?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    let payload: Option<String> = sqlx::query_scalar(
        "SELECT result::text FROM finance.rule_simulation WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3 AND id=$4",
    )
    .bind(t)
    .bind(c)
    .bind(a)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let result = payload
        .map(|json| serde_json::from_str(&json).map_err(|_| Error::Conflict))
        .transpose()?;
    recheck_authenticated(&mut tx, Some(proof)).await?;
    tx.commit().await?;
    Ok(result)
}
