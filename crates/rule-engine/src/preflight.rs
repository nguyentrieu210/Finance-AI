//! Evaluates published rule definitions against current independently
//! verified evidence. Unknown currency, coverage, or reference data cannot
//! silently become a numeric fact.
use crate::{Outcome, compiler::{self,Fact,Severity,Type,Value},repository::registry};
use finance_shared::{ids::EvidenceId,money::{Amount,Currency,Money}};
use serde::{Deserialize,Serialize};
use std::{collections::{BTreeMap,BTreeSet},sync::Arc};
use uuid::Uuid;

#[derive(Deserialize)]
struct Material {
    rules:Vec<Rule>,
    fields:Vec<Field>,
    #[serde(default)]
    financial:Vec<FinancialFact>,
    #[serde(default)]
    invoice_document_count:Option<usize>,
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
#[derive(Deserialize)]
struct FinancialFact {
    path:String,
    value:String,
    currency:Option<Currency>,
    verified:bool,
    evidence:Vec<EvidenceId>,
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
            let mut evidence=field.evidence.clone();
            let value=match ty {
                Type::Date=>compiler::parse_date(raw).ok().map(Value::Date),
                Type::Decimal=>Amount::parse(raw).ok().map(Value::Decimal),
                Type::Text if raw.len()<=256&&!raw.chars().any(char::is_control)=>Some(Value::Text(Arc::from(raw))),
                Type::Money(expected) => {
                    let currency_path=format!("{}.currency",path.split('.').next().unwrap_or_default());
                    let currency=rows.get(currency_path.as_str()).and_then(|items| {
                        let mut same=items.iter().filter(|item|item.document==field.document);
                        let item=*same.next()?;
                        if same.next().is_some()||!item.verified||item.evidence.is_empty(){return None}
                        let code=Currency::parse(item.value.as_deref()?).ok()?;
                        if code!=*expected{return None}
                        evidence.extend(item.evidence.iter().copied());
                        evidence.sort();evidence.dedup();
                        Some(code)
                    });
                    currency.and_then(|currency|Amount::parse(raw).ok().map(|amount|Value::Money(Money{amount,currency})))
                },
                _=>None,
            };
            facts.insert(path.clone(),Fact{value,verified:true,fresh:true,evidence});
        }
    }
    let invoice_paths=["invoice.number","invoice.date","invoice.supplier","invoice.net","invoice.tax","invoice.total","invoice.currency"];
    let invoice:Option<Vec<_>>=invoice_paths.iter().map(|path|rows.get(path).and_then(|rows|(rows.len()==1).then_some(rows[0]))).collect();
    if let Some(invoice)=invoice {
        if invoice.iter().all(|f|f.document==invoice[0].document&&f.verified&&f.value.is_some()&&!f.evidence.is_empty()){
            let evidence:BTreeSet<_>=invoice.iter().flat_map(|f|f.evidence.iter().copied()).collect();
            if evidence.len()<=64 {
                facts.insert("invoice.verified".into(),Fact{value:Some(Value::Boolean(true)),verified:true,fresh:true,evidence:evidence.into_iter().collect()});
            }
        }
    }
    // Never allow a document field to impersonate a cap or approved coverage.
    let mut financial_seen=BTreeSet::new();
    for row in &material.financial {
        if !financial_seen.insert(&row.path)||row.evidence.len()>64 {return Err("invalid financial facts")}
        if !row.verified||row.evidence.is_empty(){continue}
        let approved_path=matches!(row.path.as_str(),"reference.fresh"|"advance.overdue"|"matching.complete"|"matching.allocated_value"|"matching.line_count"|"settlement.extra_verified")||row.path.starts_with("finance.")||
            ["contract.","invoice.","advance.","budget."].iter().any(|prefix|row.path.starts_with(prefix)) &&
            [".remaining",".cap",".consumed",".other_holds"].iter().any(|suffix|row.path.ends_with(suffix));
        if !approved_path{ return Err("unapproved financial fact path") }
        let value=match types.get(&row.path){
            Some(Type::Money(currency)) if row.currency==Some(*currency)=>Amount::parse(&row.value).ok().map(|amount|Value::Money(Money{amount,currency:*currency})),
            Some(Type::Boolean) if row.currency.is_none()=>match row.value.as_str(){"true"=>Some(Value::Boolean(true)),"false"=>Some(Value::Boolean(false)),_=>None},
            Some(Type::Decimal) if row.currency.is_none()=>Amount::parse(&row.value).ok().map(Value::Decimal),
            _=>None,
        };
        facts.insert(row.path.clone(),Fact{value,verified:true,fresh:true,evidence:row.evidence.clone()});
    }
    // Collection is present only when every invoice amount is independently
    // verified and its currency is confirmed on the same document.
    if let Some(totals)=rows.get("invoice.total"){
        let expected=Currency::parse("VND").map_err(|_|"currency registry")?;
        let mut documents=BTreeSet::new();
        let mut values=Vec::new();let mut evidence=BTreeSet::new();let mut complete=true;
        for field in totals {
            if !documents.insert(field.document)||!field.verified||field.evidence.is_empty(){complete=false;break}
            let mut currencies=rows.get("invoice.currency").into_iter().flatten().filter(|f|f.document==field.document);
            let Some(currency)=currencies.next()else{complete=false;break};
            if currencies.next().is_some()||!currency.verified||currency.evidence.is_empty()||currency.value.as_deref()!=Some(expected.as_str()){complete=false;break}
            let Some(amount)=field.value.as_deref().and_then(|v|Amount::parse(v).ok())else{complete=false;break};
            values.push(Some(amount));evidence.extend(field.evidence.iter().copied());evidence.extend(currency.evidence.iter().copied());
        }
        if complete&&material.invoice_document_count==Some(values.len())&&!values.is_empty()&&evidence.len()<=64{
            facts.insert("invoice.totals".into(),Fact{value:Some(Value::MoneyList(expected,Arc::from(values))),verified:true,fresh:true,evidence:evidence.into_iter().collect()});
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
    fn unverified_overdue_or_matching_fact_stays_unknown(){
        let mut material=json!({"rules":[{"id":"00000000-0000-0000-0000-000000000001","name":"overdue","version":1,"severity":"BLOCK",
          "definition":{"rule_id":"overdue","version":1,"severity":"BLOCK","waivable":false,"clause_id":"00000000-0000-0000-0000-000000000002",
          "condition":{"op":"not","args":[{"ref":"advance.overdue"}]}}}],"fields":[],"financial":[{"path":"advance.overdue","value":"false","verified":false,"evidence":["00000000-0000-0000-0000-000000000003"]}]});
        assert_eq!(evaluate(&material).unwrap()[0].outcome,"UNKNOWN");
        material["financial"][0]["verified"]=json!(true);
        assert_eq!(evaluate(&material).unwrap()[0].outcome,"PASS");
        material["financial"][0]["value"]=json!("true");
        assert_eq!(evaluate(&material).unwrap()[0].outcome,"FAIL");
    }
    #[test]
    fn amount_without_currency_and_conflicting_invoice_remain_unknown(){
        let material=json!({"rules":[{"id":"00000000-0000-0000-0000-000000000001","name":"amount","version":1,"severity":"BLOCK",
            "definition":{"rule_id":"amount","version":1,"severity":"BLOCK","waivable":false,"clause_id":"00000000-0000-0000-0000-000000000002",
            "condition":{"op":"lte","args":[{"ref":"invoice.total"},{"literal":"100","value_type":"money","currency":"VND"}]}}}],
            "fields":[{"path":"invoice.total","document":"00000000-0000-0000-0000-000000000003","value":"10","verified":true,
            "evidence":["00000000-0000-0000-0000-000000000004"]}]});
        let rows=evaluate(&material).unwrap();
        assert_eq!(rows[0].outcome,"UNKNOWN");
        let mut material=material;
        material["fields"].as_array_mut().unwrap().push(json!({"path":"invoice.currency","document":"00000000-0000-0000-0000-000000000003","value":"VND","verified":true,"evidence":["00000000-0000-0000-0000-000000000005"]}));
        let rows=evaluate(&material).unwrap();
        assert_eq!(rows[0].outcome,"PASS");
        assert_eq!(rows[0].evidence.len(),2);
        material["fields"][1]["value"]=json!("USD");
        assert_eq!(evaluate(&material).unwrap()[0].outcome,"UNKNOWN");
    }
}
