use finance_rule_engine::{
    Outcome,
    compiler::{self, Error, Fact, Type, Value},
};
use finance_shared::{
    ids::EvidenceId,
    money::{Amount, Currency, Money},
};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use uuid::Uuid;

fn rule(condition: Json) -> Vec<u8> {
    serde_json::to_vec(&json!({"rule_id":"R02","version":1,"condition":condition,"severity":"BLOCK","waivable":false,"clause_id":"00000000-0000-0000-0000-000000000001"})).unwrap()
}
fn money(n: &str) -> Money {
    Money {
        amount: Amount::parse(n).unwrap(),
        currency: Currency::parse("VND").unwrap(),
    }
}
#[test]
fn exact_money_refs_and_evidence() {
    let registry = BTreeMap::from([
        (
            "payment.requested".into(),
            Type::Money(Currency::parse("VND").unwrap()),
        ),
        (
            "contract.available".into(),
            Type::Money(Currency::parse("VND").unwrap()),
        ),
    ]);
    let compiled = compiler::compile(
        &rule(
            json!({"op":"lte","args":[{"ref":"payment.requested"},{"ref":"contract.available"}]}),
        ),
        &registry,
    )
    .unwrap();
    let ev = EvidenceId::try_from(Uuid::from_u128(2)).unwrap();
    let fact = |n| Fact {
        value: Some(Value::Money(money(n))),
        verified: true,
        fresh: true,
        evidence: vec![ev],
    };
    let mut facts = BTreeMap::from([
        ("payment.requested".into(), fact("110")),
        ("contract.available".into(), fact("100")),
    ]);
    assert_eq!(compiler::evaluate(&compiled, &facts).outcome, Outcome::Fail);
    facts.insert("payment.requested".into(), fact("90"));
    let result = compiler::evaluate(&compiled, &facts);
    assert_eq!(result.outcome, Outcome::Pass);
    assert_eq!(result.evidence, vec![ev]);
    facts.get_mut("contract.available").unwrap().fresh = false;
    assert_eq!(
        compiler::evaluate(&compiled, &facts).outcome,
        Outcome::Unknown
    );
}
#[test]
fn reject_unknown_path_currency_arity_and_arbitrary_operator() {
    let registry = BTreeMap::from([("a".into(), Type::Money(Currency::parse("VND").unwrap()))]);
    let errors = [
        (json!({"ref":"missing"}), Error::UnknownReference),
        (
            json!({"op":"lte","args":[{"ref":"a"},{"literal":"1","value_type":"money","currency":"USD"}]}),
            Error::TypeMismatch,
        ),
        (json!({"op":"lte","args":[{"ref":"a"}]}), Error::WrongArity),
        (
            json!({"op":"exec","args":[{"ref":"a"}]}),
            Error::UnsupportedOperator,
        ),
    ];
    for (condition, error) in errors {
        assert_eq!(
            compiler::compile(&rule(condition), &registry).err(),
            Some(error)
        );
    }
}

fn decimal(value: &str) -> Json {
    json!({"literal": value, "value_type": "decimal"})
}

fn date(value: &str) -> Json {
    json!({"literal": value, "value_type": "date"})
}

fn call(op: &str, args: Vec<Json>) -> Json {
    json!({"op": op, "args": args})
}

fn outcome(node: Json) -> Outcome {
    let compiled = compiler::compile(&rule(node), &BTreeMap::new()).unwrap();
    compiler::evaluate(&compiled, &BTreeMap::new()).outcome
}

#[test]
fn decimal_multiply_and_divide_are_exact_or_error() {
    for (op, a, b, expected) in [
        ("mul", "0.125", "0.08", "0.01"),
        ("mul", "-2.5", "4", "-10"),
        ("mul", "0.00000005", "0.2", "0.00000001"),
        ("div", "1", "8", "0.125"),
        ("div", "0.00000001", "0.00000002", "0.5"),
        ("div", "3", "-2", "-1.5"),
        ("div", "100", "0.001", "100000"),
    ] {
        assert_eq!(
            outcome(call(
                "eq",
                vec![call(op, vec![decimal(a), decimal(b)]), decimal(expected)]
            )),
            Outcome::Pass
        );
    }
    for (op, a, b) in [
        ("div", "1", "0"),
        ("div", "1", "3"),
        ("div", "1", "512"),
        ("mul", "0.00000001", "0.1"),
        ("mul", "99999999999999999999", "2"),
    ] {
        assert_eq!(
            outcome(call(
                "eq",
                vec![call(op, vec![decimal(a), decimal(b)]), decimal("0")]
            )),
            Outcome::Error
        );
    }
}

#[test]
fn date_rules_use_calendar_days_and_reject_invalid_dates() {
    assert_eq!(
        outcome(call(
            "eq",
            vec![
                call("date_diff", vec![date("2024-03-01"), date("2024-02-28")]),
                decimal("2")
            ]
        )),
        Outcome::Pass
    );
    assert_eq!(
        outcome(call("lt", vec![date("2024-02-29"), date("2024-03-01")])),
        Outcome::Pass
    );
    for invalid in ["2023-02-29", "2024-2-09", "0000-01-01"] {
        assert_eq!(
            compiler::compile(
                &rule(call("eq", vec![date(invalid), date("2024-01-01")])),
                &BTreeMap::new()
            )
            .err(),
            Some(Error::InvalidLiteral)
        );
    }
    assert_eq!(
        compiler::compile(
            &rule(call("add", vec![date("2024-01-01"), date("2024-01-01")])),
            &BTreeMap::new()
        )
        .err(),
        Some(Error::TypeMismatch)
    );
}

#[test]
fn exists_distinguishes_verified_absence_from_unknown() {
    let registry = BTreeMap::from([("invoice.date".to_owned(), Type::Date)]);
    let exists = call("exists", vec![json!({"ref": "invoice.date"})]);
    let compiled = compiler::compile(&rule(exists.clone()), &registry).unwrap();
    let negated = compiler::compile(&rule(call("not", vec![exists])), &registry).unwrap();
    let mut facts = BTreeMap::new();
    assert_eq!(compiler::evaluate(&compiled, &facts).outcome, Outcome::Unknown);
    let evidence = EvidenceId::try_from(Uuid::from_u128(2)).unwrap();
    facts.insert(
        "invoice.date".to_owned(),
        Fact {
            value: None,
            verified: true,
            fresh: true,
            evidence: vec![evidence],
        },
    );
    assert_eq!(compiler::evaluate(&compiled, &facts).outcome, Outcome::Unknown);
    facts.get_mut("invoice.date").unwrap().value = Some(Value::Absent(Type::Date));
    let result = compiler::evaluate(&compiled, &facts);
    assert_eq!(result.outcome, Outcome::Fail);
    assert_eq!(result.evidence, vec![evidence]);
    assert_eq!(compiler::evaluate(&negated, &facts).outcome, Outcome::Pass);
    let comparison = compiler::compile(
        &rule(call("eq", vec![json!({"ref": "invoice.date"}), date("2024-01-01")])),
        &registry,
    )
    .unwrap();
    assert_eq!(compiler::evaluate(&comparison, &facts).outcome, Outcome::Unknown);
    facts.get_mut("invoice.date").unwrap().fresh = false;
    assert_eq!(compiler::evaluate(&negated, &facts).outcome, Outcome::Unknown);
    let fact = facts.get_mut("invoice.date").unwrap();
    fact.fresh = true;
    fact.value = Some(Value::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap()));
    assert_eq!(compiler::evaluate(&compiled, &facts).outcome, Outcome::Pass);
}

#[test]
fn set_membership_and_count_require_known_coverage() {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    let set = json!({"literal": ["A", "B"], "value_type": "set"});
    let a = json!({"literal": "A", "value_type": "string"});
    assert_eq!(outcome(call("in", vec![a.clone(), set.clone()])), Outcome::Pass);
    assert_eq!(
        outcome(call("eq", vec![call("count", vec![set]), decimal("2")])),
        Outcome::Pass
    );
    assert_eq!(
        outcome(call(
            "eq",
            vec![
                call("count", vec![json!({"literal": [], "value_type": "set"})]),
                decimal("0")
            ]
        )),
        Outcome::Pass
    );
    let registry = BTreeMap::from([("docs.kinds".to_owned(), Type::Set)]);
    let compiled = compiler::compile(
        &rule(call("in", vec![a, json!({"ref": "docs.kinds"})])),
        &registry,
    )
    .unwrap();
    let mut facts = BTreeMap::new();
    assert_eq!(compiler::evaluate(&compiled, &facts).outcome, Outcome::Unknown);
    facts.insert(
        "docs.kinds".to_owned(),
        Fact {
            value: Some(Value::Set(Arc::new(BTreeSet::new()))),
            verified: true,
            fresh: true,
            evidence: vec![EvidenceId::try_from(Uuid::from_u128(3)).unwrap()],
        },
    );
    assert_eq!(compiler::evaluate(&compiled, &facts).outcome, Outcome::Fail);
    facts.get_mut("docs.kinds").unwrap().fresh = false;
    assert_eq!(compiler::evaluate(&compiled, &facts).outcome, Outcome::Unknown);
}

#[test]
fn duplicate_set_items_and_cross_type_membership_rejected() {
    let bad_set = json!({"literal": ["A", "A"], "value_type": "set"});
    let text = json!({"literal": "A", "value_type": "string"});
    assert_eq!(
        compiler::compile(&rule(call("in", vec![text, bad_set])), &BTreeMap::new()).err(),
        Some(Error::InvalidLiteral)
    );
    assert_eq!(
        compiler::compile(
            &rule(call("in", vec![decimal("1"), json!({"literal": ["1"], "value_type": "set"})])),
            &BTreeMap::new()
        )
        .err(),
        Some(Error::TypeMismatch)
    );
}

#[test]
fn quantities_cannot_mix_units_or_currencies() {
    let quantity = |value: &str, unit: &str| {
        json!({"literal": value, "value_type": "quantity", "unit": unit})
    };
    assert_eq!(
        outcome(call(
            "eq",
            vec![
                call("add", vec![quantity("2", "kg"), quantity("3", "kg")]),
                quantity("5", "kg")
            ]
        )),
        Outcome::Pass
    );
    assert_eq!(
        compiler::compile(
            &rule(call("eq", vec![quantity("1", "kg"), quantity("1", "m")])),
            &BTreeMap::new()
        )
        .err(),
        Some(Error::TypeMismatch)
    );
    assert_eq!(
        compiler::compile(
            &rule(call("eq", vec![quantity("1", ""), quantity("1", "")])),
            &BTreeMap::new()
        )
        .err(),
        Some(Error::InvalidLiteral)
    );
}

#[test]
fn collection_aggregates_distinguish_empty_unknown_and_incomplete() {
    use std::sync::Arc;

    let registry = BTreeMap::from([
        ("rows.amounts".to_owned(), Type::DecimalList),
        ("rows.flags".to_owned(), Type::BooleanList),
    ]);
    let sum = compiler::compile(
        &rule(call(
            "eq",
            vec![call("sum", vec![json!({"ref": "rows.amounts"})]), decimal("5")],
        )),
        &registry,
    )
    .unwrap();
    let all = compiler::compile(
        &rule(call("all", vec![json!({"ref": "rows.flags"})])),
        &registry,
    )
    .unwrap();
    let any = compiler::compile(
        &rule(call("any", vec![json!({"ref": "rows.flags"})])),
        &registry,
    )
    .unwrap();
    let evidence = EvidenceId::try_from(Uuid::from_u128(4)).unwrap();
    let verified = |value| Fact {
        value: Some(value),
        verified: true,
        fresh: true,
        evidence: vec![evidence],
    };
    let mut facts = BTreeMap::from([
        (
            "rows.amounts".to_owned(),
            verified(Value::DecimalList(Arc::from([
                Some(Amount::parse("2").unwrap()),
                Some(Amount::parse("3").unwrap()),
            ]))),
        ),
        (
            "rows.flags".to_owned(),
            verified(Value::BooleanList(Arc::from([Some(false), None]))),
        ),
    ]);
    assert_eq!(compiler::evaluate(&sum, &facts).outcome, Outcome::Pass);
    assert_eq!(compiler::evaluate(&all, &facts).outcome, Outcome::Fail);
    assert_eq!(compiler::evaluate(&any, &facts).outcome, Outcome::Unknown);
    facts.get_mut("rows.amounts").unwrap().value =
        Some(Value::DecimalList(Arc::from([None])));
    assert_eq!(compiler::evaluate(&sum, &facts).outcome, Outcome::Unknown);
    facts.get_mut("rows.flags").unwrap().value = Some(Value::BooleanList(Arc::from([])));
    assert_eq!(compiler::evaluate(&all, &facts).outcome, Outcome::Pass);
    assert_eq!(compiler::evaluate(&any, &facts).outcome, Outcome::Fail);
    facts.get_mut("rows.flags").unwrap().verified = false;
    assert_eq!(compiler::evaluate(&all, &facts).outcome, Outcome::Unknown);
}
