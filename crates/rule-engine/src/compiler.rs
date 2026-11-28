//! Bounded typed JSON compiler for scalar and complete-collection rules.
//! Unsupported schema operators fail compilation, not evaluation to PASS.
use crate::Outcome;
use chrono::NaiveDate;
use finance_shared::{
    ids::EvidenceId,
    money::{Amount, Currency, Money},
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Exact unit identity; conversion requires a separately approved rule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Unit {
    bytes: [u8; 32],
    len: u8,
}

impl Unit {
    pub fn parse(value: &str) -> Result<Self, Error> {
        if value.is_empty()
            || value.len() > 32
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b))
        {
            return Err(Error::InvalidLiteral);
        }
        let mut bytes = [0; 32];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            len: value.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).expect("validated ASCII unit")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Type {
    Boolean,
    Money(Currency),
    Decimal,
    Date,
    Text,
    Enum,
    Set,
    Quantity(Unit),
    DecimalList,
    MoneyList(Currency),
    BooleanList,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    Boolean(bool),
    Money(Money),
    Decimal(Amount),
    Date(NaiveDate),
    Text(Arc<str>),
    Enum(Arc<str>),
    Set(Arc<BTreeSet<String>>),
    Quantity(Amount, Unit),
    DecimalList(Arc<[Option<Amount>]>),
    MoneyList(Currency, Arc<[Option<Amount>]>),
    BooleanList(Arc<[Option<bool>]>),
    /// Explicit independently verified absence, distinct from a missing fact.
    Absent(Type),
}

impl Value {
    fn ty(&self) -> Type {
        match self {
            Self::Boolean(_) => Type::Boolean,
            Self::Money(m) => Type::Money(m.currency),
            Self::Decimal(_) => Type::Decimal,
            Self::Date(_) => Type::Date,
            Self::Text(_) => Type::Text,
            Self::Enum(_) => Type::Enum,
            Self::Set(_) => Type::Set,
            Self::Quantity(_, unit) => Type::Quantity(*unit),
            Self::DecimalList(_) => Type::DecimalList,
            Self::MoneyList(currency, _) => Type::MoneyList(*currency),
            Self::BooleanList(_) => Type::BooleanList,
            Self::Absent(ty) => *ty,
        }
    }

    fn valid(&self) -> bool {
        match self {
            Self::Text(s) => valid_text(s),
            Self::Enum(s) => valid_text(s) && !s.is_empty(),
            Self::Set(items) => valid_set(items),
            Self::DecimalList(items) | Self::MoneyList(_, items) => items.len() <= 5000,
            Self::BooleanList(items) => items.len() <= 5000,
            _ => true,
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
    unit: Option<String>,
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
    Mul,
    Div,
    Exists,
    DateDiff,
    In,
    Count,
    Sum,
    All,
    Any,
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

pub(crate) fn parse_date(value: &str) -> Result<NaiveDate, Error> {
    if value.len() != 10 {
        return Err(Error::InvalidLiteral);
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| Error::InvalidLiteral)?;
    if date.format("%Y-%m-%d").to_string() != value || value.starts_with("0000") {
        return Err(Error::InvalidLiteral);
    }
    Ok(date)
}

fn valid_text(s: &str) -> bool {
    s.len() <= 256 && !s.chars().any(char::is_control)
}

fn valid_set(items: &BTreeSet<String>) -> bool {
    items.len() <= 5000
        && items.iter().all(|s| valid_text(s))
        && items.iter().map(String::len).sum::<usize>() <= 65536
}

fn literal(raw: Literal) -> Result<Value, Error> {
    if raw.value_type == "quantity" {
        return match (raw.literal, raw.currency, raw.unit) {
            (serde_json::Value::String(s), None, Some(unit)) if s.len() <= 32 => {
                Ok(Value::Quantity(
                    Amount::parse(&s).map_err(|_| Error::InvalidLiteral)?,
                    Unit::parse(&unit)?,
                ))
            }
            _ => Err(Error::InvalidLiteral),
        };
    }
    if raw.unit.is_some() {
        return Err(Error::InvalidLiteral);
    }
    match (raw.value_type.as_str(), raw.literal, raw.currency) {
        ("boolean", serde_json::Value::Bool(b), None) => Ok(Value::Boolean(b)),
        ("date", serde_json::Value::String(s), None) => Ok(Value::Date(parse_date(&s)?)),
        ("string", serde_json::Value::String(s), None) if valid_text(&s) => {
            Ok(Value::Text(Arc::from(s)))
        }
        ("enum", serde_json::Value::String(s), None) if valid_text(&s) && !s.is_empty() => {
            Ok(Value::Enum(Arc::from(s)))
        }
        ("set", serde_json::Value::Array(items), None) if items.len() <= 5000 => {
            let mut set = BTreeSet::new();
            for item in items {
                let serde_json::Value::String(s) = item else {
                    return Err(Error::InvalidLiteral);
                };
                if !valid_text(&s) || !set.insert(s) {
                    return Err(Error::InvalidLiteral);
                }
            }
            if !valid_set(&set) {
                return Err(Error::InvalidLiteral);
            }
            Ok(Value::Set(Arc::new(set)))
        }
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
                "mul" => Op::Mul,
                "div" => Op::Div,
                "exists" => Op::Exists,
                "date_diff" => Op::DateDiff,
                "in" => Op::In,
                "count" => Op::Count,
                "sum" => Op::Sum,
                "all" => Op::All,
                "any" => Op::Any,
                _ => return Err(Error::UnsupportedOperator),
            };
            let arity = c.args.len();
            if !(1..=16).contains(&arity)
                || match op {
                    Op::Not | Op::Exists | Op::Count | Op::Sum | Op::All | Op::Any => arity != 1,
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
            if matches!(op, Op::In) {
                if !matches!(first, Type::Text | Type::Enum) || args[1].ty() != Type::Set {
                    return Err(Error::TypeMismatch);
                }
            } else if args.iter().any(|a| a.ty() != first) {
                return Err(Error::TypeMismatch);
            }
            let ty = match op {
                Op::And | Op::Or | Op::Not if first == Type::Boolean => Type::Boolean,
                Op::Eq | Op::Ne
                    if !matches!(
                        first,
                        Type::DecimalList | Type::MoneyList(_) | Type::BooleanList
                    ) =>
                {
                    Type::Boolean
                }
                Op::Lt | Op::Lte | Op::Gt | Op::Gte
                    if matches!(
                        first,
                        Type::Money(_) | Type::Decimal | Type::Date | Type::Quantity(_)
                    ) =>
                {
                    Type::Boolean
                }
                Op::Add | Op::Sub
                    if matches!(first, Type::Money(_) | Type::Decimal | Type::Quantity(_)) =>
                {
                    first
                }
                Op::Mul | Op::Div if first == Type::Decimal => Type::Decimal,
                Op::Exists => Type::Boolean,
                Op::DateDiff if first == Type::Date => Type::Decimal,
                Op::In => Type::Boolean,
                Op::Count
                    if matches!(
                        first,
                        Type::Set | Type::DecimalList | Type::MoneyList(_) | Type::BooleanList
                    ) =>
                {
                    Type::Decimal
                }
                Op::Sum if first == Type::DecimalList => Type::Decimal,
                Op::Sum if matches!(first, Type::MoneyList(_)) => match first {
                    Type::MoneyList(currency) => Type::Money(currency),
                    _ => unreachable!(),
                },
                Op::All | Op::Any if first == Type::BooleanList => Type::Boolean,
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

#[derive(Clone, Eq, PartialEq)]
enum Eval {
    Known(Value),
    Absent,
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
        Node::Literal(v) => Known(v.clone()),
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
            match &f.value {
                Some(Value::Absent(absent_type)) if absent_type == ty => Absent,
                Some(value) if value.ty() == *ty && value.valid() => Known(value.clone()),
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
                    } else if values.contains(&Unknown) || values.contains(&Absent) {
                        Unknown
                    } else {
                        Known(Value::Boolean(!dominant))
                    }
                }
                Op::Not => match &values[0] {
                    Known(Value::Boolean(b)) => Known(Value::Boolean(!*b)),
                    Unknown | Absent => Unknown,
                    _ => Error,
                },
                Op::Exists => match &values[0] {
                    Known(_) => Known(Value::Boolean(true)),
                    Absent => Known(Value::Boolean(false)),
                    Unknown => Unknown,
                    Error => Error,
                },
                Op::Count | Op::Sum | Op::All | Op::Any => match &values[0] {
                    Known(value) => aggregate(*op, value),
                    Unknown | Absent => Unknown,
                    Error => Error,
                },
                _ => {
                    let [Known(a), Known(b)] = values.as_slice() else {
                        return Unknown;
                    };
                    binary(*op, a, b)
                }
            }
        }
    }
}

fn aggregate(op: Op, value: &Value) -> Eval {
    use Eval::*;
    if matches!(op, Op::Count) {
        let count = match value {
            Value::Set(items) => items.len(),
            Value::DecimalList(items) | Value::MoneyList(_, items) => items.len(),
            Value::BooleanList(items) => items.len(),
            _ => return Error,
        };
        return match Amount::parse(&count.to_string()) {
            Ok(amount) => Known(Value::Decimal(amount)),
            Err(_) => Error,
        };
    }
    if matches!(op, Op::All | Op::Any) {
        let Value::BooleanList(items) = value else {
            return Error;
        };
        let dominant = matches!(op, Op::Any);
        return if items.contains(&Some(dominant)) {
            Known(Value::Boolean(dominant))
        } else if items.contains(&None) {
            Unknown
        } else {
            Known(Value::Boolean(!dominant))
        };
    }
    let (items, currency) = match value {
        Value::DecimalList(items) => (items, None),
        Value::MoneyList(currency, items) => (items, Some(*currency)),
        _ => return Error,
    };
    let mut total = Amount::parse("0").expect("zero is representable");
    let mut unknown = false;
    for item in items.iter() {
        match item {
            Some(amount) => match total.checked_add(*amount) {
                Ok(next) => total = next,
                Err(_) => return Error,
            },
            None => unknown = true,
        }
    }
    if unknown {
        return Unknown;
    }
    Known(match currency {
        Some(currency) => Value::Money(Money {
            amount: total,
            currency,
        }),
        None => Value::Decimal(total),
    })
}

fn binary(op: Op, a: &Value, b: &Value) -> Eval {
    use Eval::*;
    if matches!(op, Op::Eq | Op::Ne) {
        return Known(Value::Boolean(if matches!(op, Op::Eq) {
            a == b
        } else {
            a != b
        }));
    }
    if matches!(op, Op::In) {
        return match (a, b) {
            (Value::Text(s) | Value::Enum(s), Value::Set(items)) => {
                Known(Value::Boolean(items.contains(s.as_ref())))
            }
            _ => Error,
        };
    }
    if let (Value::Date(x), Value::Date(y)) = (a, b) {
        return match op {
            Op::Lt => Known(Value::Boolean(x < y)),
            Op::Lte => Known(Value::Boolean(x <= y)),
            Op::Gt => Known(Value::Boolean(x > y)),
            Op::Gte => Known(Value::Boolean(x >= y)),
            Op::DateDiff => {
                match Amount::parse(&x.signed_duration_since(*y).num_days().to_string()) {
                    Ok(amount) => Known(Value::Decimal(amount)),
                    Err(_) => Error,
                }
            }
            _ => Error,
        };
    }
    if matches!(op, Op::Mul | Op::Div) {
        return match (a, b) {
            (Value::Decimal(x), Value::Decimal(y)) => match exact_decimal(op, *x, *y) {
                Some(amount) => Known(Value::Decimal(amount)),
                None => Error,
            },
            _ => Error,
        };
    }
    let (x, y) = match (a, b) {
        (Value::Money(x), Value::Money(y)) if x.currency == y.currency => (x.amount, y.amount),
        (Value::Decimal(x), Value::Decimal(y)) => (*x, *y),
        (Value::Quantity(x, ux), Value::Quantity(y, uy)) if ux == uy => (*x, *y),
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
                    Value::Quantity(_, unit) => Value::Quantity(amount, *unit),
                    _ => Value::Decimal(amount),
                }),
                Err(_) => Error,
            }
        }
        _ => Error,
    }
}

// Work with integer mantissas, never an implicitly rounded decimal result.
fn amount_from_parts(mut n: i128, mut scale: i32) -> Option<Amount> {
    if n == 0 {
        return Amount::parse("0").ok();
    }
    while scale > 0 && n % 10 == 0 {
        n /= 10;
        scale -= 1;
    }
    if scale < 0 {
        n = n.checked_mul(10_i128.checked_pow((-scale) as u32)?)?;
        scale = 0;
    }
    if scale > 8 {
        return None;
    }
    let sign = if n < 0 { "-" } else { "" };
    let digits = n.unsigned_abs().to_string();
    let s = scale as usize;
    let text = if s == 0 {
        format!("{sign}{digits}")
    } else if digits.len() <= s {
        format!("{sign}0.{}{digits}", "0".repeat(s - digits.len()))
    } else {
        let (integer, fraction) = digits.split_at(digits.len() - s);
        format!("{sign}{integer}.{fraction}")
    };
    Amount::parse(&text).ok()
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn exact_decimal(op: Op, x: Amount, y: Amount) -> Option<Amount> {
    let x = x.decimal();
    let y = y.decimal();
    let mut a = x.mantissa();
    let mut b = y.mantissa();
    match op {
        Op::Mul => {
            let mut scale = (x.scale() + y.scale()) as i32;
            if a == 0 || b == 0 {
                return Amount::parse("0").ok();
            }
            // Cancel powers of ten before multiplying to avoid intermediate overflow.
            while scale > 0 {
                if a % 10 == 0 {
                    a /= 10;
                } else if b % 10 == 0 {
                    b /= 10;
                } else if a % 2 == 0 && b % 5 == 0 {
                    a /= 2;
                    b /= 5;
                } else if a % 5 == 0 && b % 2 == 0 {
                    a /= 5;
                    b /= 2;
                } else {
                    break;
                }
                scale -= 1;
            }
            amount_from_parts(a.checked_mul(b)?, scale)
        }
        Op::Div => {
            if b == 0 {
                return None;
            }
            if a == 0 {
                return Amount::parse("0").ok();
            }
            let divisor = gcd(a.abs(), b.abs());
            a /= divisor;
            b /= divisor;
            if b < 0 {
                a = -a;
                b = -b;
            }
            let mut twos = 0_u32;
            let mut fives = 0_u32;
            while b % 2 == 0 {
                b /= 2;
                twos += 1;
            }
            while b % 5 == 0 {
                b /= 5;
                fives += 1;
            }
            if b != 1 {
                return None; // Nonterminating decimal: no approved rounding policy.
            }
            let digits = twos.max(fives);
            let mut scale = digits as i32 + x.scale() as i32 - y.scale() as i32;
            while scale > 0 && a % 10 == 0 {
                a /= 10;
                scale -= 1;
            }
            if scale > 8 {
                return None;
            }
            a = a.checked_mul(2_i128.checked_pow(digits - twos)?)?;
            a = a.checked_mul(5_i128.checked_pow(digits - fives)?)?;
            amount_from_parts(a, scale)
        }
        _ => None,
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
        Some(Eval::Unknown | Eval::Absent) => Outcome::Unknown,
        Some(Eval::Error) => Outcome::Error,
        None | Some(Eval::Known(Value::Boolean(true))) => match eval(
            &compiled.condition,
            facts,
            &mut evidence,
            &mut evaluated_refs,
        ) {
            Eval::Known(Value::Boolean(true)) => Outcome::Pass,
            Eval::Known(Value::Boolean(false)) => Outcome::Fail,
            Eval::Unknown | Eval::Absent => Outcome::Unknown,
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
