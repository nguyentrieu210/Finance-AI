//! Evaluates published rule definitions against current independently
//! verified evidence. Unknown currency, coverage, or reference data cannot
//! silently become a numeric fact.
use crate::{Outcome, compiler::{self,Fact,Severity,Type,Value},repository::registry};
use finance_shared::ids::EvidenceId;
use serde::{Deserialize,Serialize};
use std::{collections::{BTreeMap,BTreeSet},sync::Arc};
use uuid::Uuid;

#[derive(Deserialize)]
struct Material {
    rules:Vec<Rule>,
    fields:Vec<Field>,
}
#[derive(Deserialize)]
struct Rule {
    id:Uuid,
    name:String,
    version:u64,
    severity:String,
    definition:serde_json::Value,
}
#[derive(Deserialize)]
struct Field {
    path:String,
    document:Uuid,
    value:Option<String>,
    evidence:Vec<EvidenceId>,
    verified:bool,
}
#[derive(Serialize)]
pub struct ResultRow {
    id:Uuid,
    version:u64,
    outcome:&'static str,
    evidence:Vec<EvidenceId>,
    detail:&'static str,
}

pub fn evaluate(material:&serde_json::Value)->Result<Vec<ResultRow>, &'static str>{
    let material:Material=serde_json::from_value(material.clone()).map_err(|_|"invalid material")?;
    if material.rules.len()>1000||material.fields.len()>5000{return Err("material limit")}
    let types=registry();
    let mut rows:BTreeMap<&str,Vec<&Field>>=BTreeMap::new();
    for field in &material.fields {
        if field.evidence.len()>64||field.path.len()>128{return Err("invalid field evidence")}
        rows.entry(&field.path).or_default().push(field);
    }
    let mut facts=BTreeMap::new();
    for (path,ty) in &types {
        if let Some(fields)=rows.get(path.as_str()) {
            // Multiple documents with a scalar path are ambiguous, not a sum.
            if fields.len()!=1{continue}
            let field=fields[0];
            if !field.verified||field.evidence.is_empty(){continue}
            let Some(raw)=field.value.as_deref()else{continue};
            let value=match ty {
                Type::Date=>compiler::parse_date(raw).ok().map(Value::Date),
                Type::Text if raw.len()<=256&&!raw.chars().any(char::is_control)=>Some(Value::Text(Arc::from(raw))),
                // No authoritative currency/coverage projection is present in
                // the dossier field schema. Do not reinterpret an amount as VND.
                _=>None,
            };
            facts.insert(path.clone(),Fact{value,verified:true,fresh:true,evidence:field.evidence.clone()});
        }
    }
    let invoice_paths=["invoice.number","invoice.date","invoice.supplier","invoice.net","invoice.tax","invoice.total"];
    let invoice:Option<Vec<_>>=invoice_paths.iter().map(|path|rows.get(path).and_then(|rows|(rows.len()==1).then_some(rows[0]))).collect();
    if let Some(invoice)=invoice {
        if invoice.iter().all(|f|f.document==invoice[0].document&&f.verified&&f.value.is_some()&&!f.evidence.is_empty()){
            let evidence:BTreeSet<_>=invoice.iter().flat_map(|f|f.evidence.iter().copied()).collect();
            if evidence.len()<=64 {
                facts.insert("invoice.verified".into(),Fact{value:Some(Value::Boolean(true)),verified:true,fresh:true,evidence:evidence.into_iter().collect()});
            }
        }
    }
    let mut seen=BTreeSet::new();
    let mut result=Vec::with_capacity(material.rules.len());
    for rule in material.rules {
        if !seen.insert(rule.id)||rule.id.is_nil()||rule.version==0{return Err("duplicate or invalid rule")}
        let compiled=serde_json::to_vec(&rule.definition).ok().and_then(|bytes|compiler::compile(&bytes,&types).ok());
        let Some(compiled)=compiled.filter(|c|c.rule_id()==rule.name&&c.version()==rule.version&&(match c.severity(){Severity::Block=>"BLOCK",Severity::Warning=>"WARNING",Severity::Info=>"INFO"})==rule.severity) else {
            result.push(ResultRow{id:rule.id,version:rule.version,outcome:"ERROR",evidence:vec![],detail:"RULE_COMPILE_ERROR"});
            continue;
        };
        let evaluation=compiler::evaluate(&compiled,&facts);
        let outcome=match evaluation.outcome {
            Outcome::Pass=>"PASS",Outcome::Fail=>"FAIL",Outcome::Unknown=>"UNKNOWN",Outcome::Error=>"ERROR",Outcome::NotApplicable=>"NOT_APPLICABLE",
        };
        if evaluation.evidence.len()>256{return Err("rule evidence limit")}
        result.push(ResultRow{id:rule.id,version:rule.version,outcome,evidence:evaluation.evidence,detail:"RULE_EVALUATED"});
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn amount_without_currency_and_conflicting_invoice_remain_unknown(){
        let material=json!({"rules":[{"id":"00000000-0000-0000-0000-000000000001","name":"amount","version":1,"severity":"BLOCK",
            "definition":{"rule_id":"amount","version":1,"severity":"BLOCK","waivable":false,"clause_id":"00000000-0000-0000-0000-000000000002",
            "condition":{"op":"lte","args":[{"ref":"invoice.total"},{"literal":"100","value_type":"money","currency":"VND"}]}}}],
            "fields":[{"path":"invoice.total","document":"00000000-0000-0000-0000-000000000003","value":"10","verified":true,
            "evidence":["00000000-0000-0000-0000-000000000004"]}]});
        let rows=evaluate(&material).unwrap();
        assert_eq!(rows[0].outcome,"UNKNOWN");
    }
}
