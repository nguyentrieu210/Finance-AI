use finance_rule_engine::{Outcome, compiler::{self, Error, Fact, Type, Value}};
use finance_shared::{ids::EvidenceId,money::{Amount,Currency,Money}};
use serde_json::{Value as Json,json};
use std::collections::BTreeMap;
use uuid::Uuid;

fn rule(condition:Json)->Vec<u8>{
    serde_json::to_vec(&json!({"rule_id":"R02","version":1,"condition":condition,"severity":"BLOCK","waivable":false,"clause_id":"00000000-0000-0000-0000-000000000001"})).unwrap()
}
fn money(n:&str)->Money{Money{amount:Amount::parse(n).unwrap(),currency:Currency::parse("VND").unwrap()}}
#[test]
fn exact_money_refs_and_evidence(){
    let registry=BTreeMap::from([("payment.requested".into(),Type::Money(Currency::parse("VND").unwrap())),("contract.available".into(),Type::Money(Currency::parse("VND").unwrap()))]);
    let compiled=compiler::compile(&rule(json!({"op":"lte","args":[{"ref":"payment.requested"},{"ref":"contract.available"}]})),&registry).unwrap();
    let ev=EvidenceId::try_from(Uuid::from_u128(2)).unwrap();
    let fact=|n|Fact{value:Some(Value::Money(money(n))),verified:true,fresh:true,evidence:vec![ev]};
    let mut facts=BTreeMap::from([("payment.requested".into(),fact("110")),("contract.available".into(),fact("100"))]);
    assert_eq!(compiler::evaluate(&compiled,&facts).outcome,Outcome::Fail);
    facts.insert("payment.requested".into(),fact("90"));
    let result=compiler::evaluate(&compiled,&facts);
    assert_eq!(result.outcome,Outcome::Pass);
    assert_eq!(result.evidence,vec![ev]);
    facts.get_mut("contract.available").unwrap().fresh=false;
    assert_eq!(compiler::evaluate(&compiled,&facts).outcome,Outcome::Unknown);
}
#[test]
fn reject_unknown_path_currency_arity_and_arbitrary_operator(){
    let registry=BTreeMap::from([("a".into(),Type::Money(Currency::parse("VND").unwrap()))]);
    let errors=[
        (json!({"ref":"missing"}),Error::UnknownReference),
        (json!({"op":"lte","args":[{"ref":"a"},{"literal":"1","value_type":"money","currency":"USD"}]}),Error::TypeMismatch),
        (json!({"op":"lte","args":[{"ref":"a"}]}),Error::WrongArity),
        (json!({"op":"exec","args":[{"ref":"a"}]}),Error::UnsupportedOperator),
    ];
    for (condition,error) in errors{assert_eq!(compiler::compile(&rule(condition),&registry).err(),Some(error));}
}
