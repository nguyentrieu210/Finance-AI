//! Bounded typed JSON compiler for the scalar boolean/money/decimal subset.
//! Unsupported schema operators fail compilation, not evaluation to PASS.
use crate::Outcome;
use finance_shared::{
    ids::EvidenceId,
    money::{Amount, Currency, Money},
};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Type {
    Boolean,
    Money(Currency),
    Decimal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Value {
    Boolean(bool),
    Money(Money),
    Decimal(Amount),
}

impl Value {
    fn ty(self) -> Type {
        match self {
            Self::Boolean(_) => Type::Boolean,
            Self::Money(m) => Type::Money(m.currency),
            Self::Decimal(_) => Type::Decimal,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Fact {
    pub value: Option<Value>,
    pub verified: bool,
    pub fresh: bool,
    pub evidence: Vec<EvidenceId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidJson,
    TooLarge,
    UnknownReference,
    InvalidLiteral,
    UnsupportedOperator,
    WrongArity,
    TypeMismatch,
    InvalidRule,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Severity {
    Block,
    Warning,
    Info,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    rule_id: String,
    version: u64,
    condition: RawNode,
    applicability: Option<RawNode>,
    severity: Severity,
    waivable: bool,
    clause_id: uuid::Uuid,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawNode {
    Ref(Reference),
    Literal(Literal),
    Call(Call),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    #[serde(rename = "ref")]
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Literal {
    literal: serde_json::Value,
    value_type: String,
    currency: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    op: String,
    args: Vec<RawNode>,
}

#[derive(Clone, Copy)]
enum Op {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
    And,
    Or,
    Not,
    Add,
    Sub,
}

enum Node {
    Ref(String, Type),
    Literal(Value),
    Call(Op, Vec<Node>, Type),
}

impl Node {
    fn ty(&self) -> Type {
        match self {
            Self::Ref(_, ty) | Self::Call(_, _, ty) => *ty,
            Self::Literal(value) => value.ty(),
        }
    }
}

pub struct Compiled {
    rule_id: String,
    version: u64,
    clause_id: uuid::Uuid,
    severity: Severity,
    waivable: bool,
    condition: Node,
    applicability: Option<Node>,
}

impl Compiled {
    pub fn rule_id(&self) -> &str {
        &self.rule_id
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn clause_id(&self) -> uuid::Uuid {
        self.clause_id
    }
    pub fn severity(&self) -> Severity {
        self.severity
    }
    pub fn waivable(&self) -> bool {
        self.waivable
    }
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 128
        && path.as_bytes()[0].is_ascii_alphabetic()
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._".contains(&b))
        && path.split('.').all(|p| !p.is_empty())
}

fn literal(raw: Literal) -> Result<Value, Error> {
    match (raw.value_type.as_str(), raw.literal, raw.currency) {
        ("boolean", serde_json::Value::Bool(b), None) => Ok(Value::Boolean(b)),
        ("decimal", serde_json::Value::String(s), None) if s.len() <= 32 => Ok(Value::Decimal(
            Amount::parse(&s).map_err(|_| Error::InvalidLiteral)?,
        )),
        ("money", serde_json::Value::String(s), Some(currency)) if s.len() <= 32 => {
            Ok(Value::Money(Money {
                amount: Amount::parse(&s).map_err(|_| Error::InvalidLiteral)?,
                currency: Currency::parse(&currency).map_err(|_| Error::InvalidLiteral)?,
            }))
        }
        _ => Err(Error::InvalidLiteral),
    }
}

fn compile_node(
    raw: RawNode,
    registry: &BTreeMap<String, Type>,
    depth: usize,
    nodes: &mut usize,
) -> Result<Node, Error> {
    *nodes += 1;
    if depth > 16 || *nodes > 256 {
        return Err(Error::TooLarge);
    }
    match raw {
        RawNode::Ref(r) => {
            if !valid_path(&r.path) {
                return Err(Error::UnknownReference);
            }
            let ty = *registry.get(&r.path).ok_or(Error::UnknownReference)?;
            Ok(Node::Ref(r.path, ty))
        }
        RawNode::Literal(raw) => Ok(Node::Literal(literal(raw)?)),
        RawNode::Call(c) => {
            let op = match c.op.as_str() {
                "eq" => Op::Eq,
                "ne" => Op::Ne,
                "lt" => Op::Lt,
                "lte" => Op::Lte,
                "gt" => Op::Gt,
                "gte" => Op::Gte,
                "and" => Op::And,
                "or" => Op::Or,
                "not" => Op::Not,
                "add" => Op::Add,
                "sub" => Op::Sub,
                _ => return Err(Error::UnsupportedOperator),
            };
            let arity = c.args.len();
            if !(1..=16).contains(&arity)
                || match op {
                    Op::Not => arity != 1,
                    Op::And | Op::Or => arity < 2,
                    _ => arity != 2,
                }
            {
                return Err(Error::WrongArity);
            }
            let args = c
                .args
                .into_iter()
                .map(|a| compile_node(a, registry, depth + 1, nodes))
                .collect::<Result<Vec<_>, _>>()?;
            let first = args[0].ty();
            if args.iter().any(|a| a.ty() != first) {
                return Err(Error::TypeMismatch);
            }
            let ty = match op {
                Op::And | Op::Or | Op::Not if first == Type::Boolean => Type::Boolean,
                Op::Eq | Op::Ne => Type::Boolean,
                Op::Lt | Op::Lte | Op::Gt | Op::Gte if first != Type::Boolean => Type::Boolean,
                Op::Add | Op::Sub if first != Type::Boolean => first,
                _ => return Err(Error::TypeMismatch),
            };
            Ok(Node::Call(op, args, ty))
        }
    }
}

pub fn compile(json: &[u8], registry: &BTreeMap<String, Type>) -> Result<Compiled, Error> {
    if json.is_empty() || json.len() > 65536 || registry.len() > 5000 {
        return Err(Error::TooLarge);
    }
    let rule: RawRule = serde_json::from_slice(json).map_err(|_| Error::InvalidJson)?;
    if !valid_path(&rule.rule_id)
        || rule.version == 0
        || rule.version > 9_007_199_254_740_991
        || rule.clause_id.is_nil()
    {
        return Err(Error::InvalidRule);
    }
    let mut nodes = 0;
    let condition = compile_node(rule.condition, registry, 1, &mut nodes)?;
    let applicability = rule
        .applicability
        .map(|a| compile_node(a, registry, 1, &mut nodes))
        .transpose()?;
    if condition.ty() != Type::Boolean
        || applicability
            .as_ref()
            .is_some_and(|a| a.ty() != Type::Boolean)
    {
        return Err(Error::TypeMismatch);
    }
    Ok(Compiled {
        rule_id: rule.rule_id,
        version: rule.version,
        clause_id: rule.clause_id,
        severity: rule.severity,
        waivable: rule.waivable,
        condition,
        applicability,
    })
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Eval {
    Known(Value),
    Unknown,
    Error,
}

pub struct Evaluation {
    pub outcome: Outcome,
    pub evidence: Vec<EvidenceId>,
    pub evaluated_refs: Vec<String>,
}

fn eval(
    node: &Node,
    facts: &BTreeMap<String, Fact>,
    evidence: &mut Vec<EvidenceId>,
    refs: &mut Vec<String>,
) -> Eval {
    use Eval::*;
    match node {
        Node::Literal(v) => Known(*v),
        Node::Ref(path, ty) => {
            refs.push(path.clone());
            let Some(f) = facts.get(path) else {
                return Unknown;
            };
            if f.evidence.len() > 64 {
                return Error;
            }
            evidence.extend(&f.evidence);
            if !f.verified || !f.fresh || f.evidence.is_empty() {
                return Unknown;
            }
            match f.value {
                Some(value) if value.ty() == *ty => Known(value),
                Some(_) => Error,
                None => Unknown,
            }
        }
        Node::Call(op, args, _) => {
            let values: Vec<_> = args
                .iter()
                .map(|n| eval(n, facts, evidence, refs))
                .collect();
            if values.contains(&Error) {
                return Error;
            }
            match op {
                Op::And | Op::Or => {
                    let dominant = matches!(op, Op::Or);
                    if values.contains(&Known(Value::Boolean(dominant))) {
                        Known(Value::Boolean(dominant))
                    } else if values.contains(&Unknown) {
                        Unknown
                    } else {
                        Known(Value::Boolean(!dominant))
                    }
                }
                Op::Not => match values[0] {
                    Known(Value::Boolean(b)) => Known(Value::Boolean(!b)),
                    Unknown => Unknown,
                    _ => Error,
                },
                _ => {
                    let [Known(a), Known(b)] = values.as_slice() else {
                        return Unknown;
                    };
                    binary(*op, *a, *b)
                }
            }
        }
    }
}

fn binary(op: Op, a: Value, b: Value) -> Eval {
    use Eval::*;
    if matches!(op, Op::Eq | Op::Ne) {
        return Known(Value::Boolean(if matches!(op, Op::Eq) {
            a == b
        } else {
            a != b
        }));
    }
    let (x, y) = match (a, b) {
        (Value::Money(x), Value::Money(y)) if x.currency == y.currency => (x.amount, y.amount),
        (Value::Decimal(x), Value::Decimal(y)) => (x, y),
        _ => return Error,
    };
    let x_dec = x.decimal();
    let y_dec = y.decimal();
    match op {
        Op::Lt => Known(Value::Boolean(x_dec < y_dec)),
        Op::Lte => Known(Value::Boolean(x_dec <= y_dec)),
        Op::Gt => Known(Value::Boolean(x_dec > y_dec)),
        Op::Gte => Known(Value::Boolean(x_dec >= y_dec)),
        Op::Add | Op::Sub => {
            let result = if matches!(op, Op::Add) {
                x.checked_add(y)
            } else {
                x.checked_sub(y)
            };
            match result {
                Ok(amount) => Known(match a {
                    Value::Money(m) => Value::Money(Money {
                        amount,
                        currency: m.currency,
                    }),
                    _ => Value::Decimal(amount),
                }),
                Err(_) => Error,
            }
        }
        _ => Error,
    }
}

pub fn evaluate(compiled: &Compiled, facts: &BTreeMap<String, Fact>) -> Evaluation {
    let mut evidence = Vec::new();
    let mut evaluated_refs = Vec::new();
    let applicable = compiled
        .applicability
        .as_ref()
        .map(|n| eval(n, facts, &mut evidence, &mut evaluated_refs));
    let outcome = match applicable {
        Some(Eval::Known(Value::Boolean(false))) => Outcome::NotApplicable,
        Some(Eval::Unknown) => Outcome::Unknown,
        Some(Eval::Error) => Outcome::Error,
        None | Some(Eval::Known(Value::Boolean(true))) => match eval(
            &compiled.condition,
            facts,
            &mut evidence,
            &mut evaluated_refs,
        ) {
            Eval::Known(Value::Boolean(true)) => Outcome::Pass,
            Eval::Known(Value::Boolean(false)) => Outcome::Fail,
            Eval::Unknown => Outcome::Unknown,
            _ => Outcome::Error,
        },
        _ => Outcome::Error,
    };
    evidence.sort();
    evidence.dedup();
    evaluated_refs.sort();
    evaluated_refs.dedup();
    Evaluation {
        outcome,
        evidence,
        evaluated_refs,
    }
}
