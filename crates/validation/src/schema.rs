//! Deterministic field validation and dependency invalidation (synthetic domain).
use crate::Outcome;
use finance_shared::{money::Amount, upload_contract::Sha256Digest};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldType<'a> {
    Date,
    Digest,
    Decimal { scale: u32, signed: bool },
    Enum(&'a [&'a str]),
    Text { max_bytes: usize },
}

/// Strict Gregorian ISO calendar date, no timezone or ambiguous local format.
pub fn valid_date(value: &str) -> bool {
    let b = value.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !b
            .iter()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
    {
        return false;
    }
    let year = value[..4].parse::<u32>().unwrap_or(0);
    let month = value[5..7].parse::<u32>().unwrap_or(0);
    let day = value[8..].parse::<u32>().unwrap_or(0);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    year > 0 && day > 0 && day <= max
}

/// Missing, stale and unverified values remain UNKNOWN, even if syntactically valid.
pub fn validate(kind: FieldType<'_>, value: Option<&str>, verified: bool, fresh: bool) -> Outcome {
    let Some(value) = value.filter(|_| verified && fresh) else {
        return Outcome::Unknown;
    };
    if value.len() > 65536 {
        return Outcome::Fail;
    }
    let valid = match kind {
        FieldType::Date => valid_date(value),
        FieldType::Digest => Sha256Digest::parse(value).is_ok(),
        FieldType::Decimal { scale, signed } => {
            if scale > 8 {
                return Outcome::Error;
            }
            let input_scale = value.split_once('.').map_or(0, |(_, f)| f.len());
            input_scale <= scale as usize
                && Amount::parse(value)
                    .is_ok_and(|a| signed || !a.decimal().is_sign_negative())
        }
        FieldType::Enum(choices) => {
            if choices.is_empty() || choices.len() > 5000 {
                return Outcome::Error;
            }
            choices.contains(&value)
        }
        FieldType::Text { max_bytes } => {
            if max_bytes == 0 || max_bytes > 65536 {
                return Outcome::Error;
            }
            !value.trim().is_empty()
                && value.len() <= max_bytes
                && !value.chars().any(char::is_control)
        }
    };
    if valid {
        Outcome::Pass
    } else {
        Outcome::Fail
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphError {
    InvalidPath,
    TooLarge,
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 128
        && path.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}

/// An explicit dependency graph maps versioned source keys to downstream checks.
/// Cycles are safe: traversal visits each key at most once. No live DB mutation.
pub struct Dependencies {
    edges: BTreeMap<String, BTreeSet<String>>,
}

impl Dependencies {
    pub fn new(edges: &[(&str, &str)]) -> Result<Self, GraphError> {
        if edges.len() > 5000 {
            return Err(GraphError::TooLarge);
        }
        let mut graph = BTreeMap::<String, BTreeSet<String>>::new();
        for (source, target) in edges {
            if !valid_path(source) || !valid_path(target) {
                return Err(GraphError::InvalidPath);
            }
            graph
                .entry((*source).into())
                .or_default()
                .insert((*target).into());
        }
        Ok(Self { edges: graph })
    }

    pub fn affected(&self, changed: &[&str]) -> Result<BTreeSet<String>, GraphError> {
        if changed.len() > 5000 {
            return Err(GraphError::TooLarge);
        }
        let mut seen = BTreeSet::new();
        let mut pending = Vec::new();
        for key in changed {
            if !valid_path(key) {
                return Err(GraphError::InvalidPath);
            }
            if seen.insert((*key).to_owned()) {
                pending.push((*key).to_owned());
            }
        }
        while let Some(key) = pending.pop() {
            if let Some(targets) = self.edges.get(&key) {
                for target in targets {
                    if seen.insert(target.clone()) {
                        pending.push(target.clone());
                    }
                }
            }
        }
        Ok(seen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_scale_digest_and_verified_source() {
        assert!(valid_date("2000-02-29"));
        for bad in [
            "1900-02-29",
            "2027-02-29",
            "2026-04-31",
            "0000-01-01",
            "26-01-01",
        ] {
            assert!(!valid_date(bad));
        }
        let money = FieldType::Decimal {
            scale: 2,
            signed: false,
        };
        assert_eq!(validate(money, Some("1.01"), true, true), Outcome::Pass);
        assert_eq!(validate(money, Some("1.001"), true, true), Outcome::Fail);
        assert_eq!(validate(money, Some("-1"), true, true), Outcome::Fail);
        assert_eq!(validate(money, Some("1"), true, false), Outcome::Unknown);
        assert_eq!(
            validate(FieldType::Digest, Some("bad"), true, true),
            Outcome::Fail
        );
    }

    #[test]
    fn invalidates_transitive_checks_even_when_graph_contains_cycle() {
        let graph = Dependencies::new(&[
            ("invoice.source", "invoice.total"),
            ("invoice.total", "check.reconcile"),
            ("check.reconcile", "review.snapshot"),
            ("review.snapshot", "invoice.total"),
        ])
        .unwrap();
        let affected = graph.affected(&["invoice.source"]).unwrap();
        assert_eq!(affected.len(), 4);
        assert!(affected.contains("review.snapshot"));
        assert!(!affected.contains("other.company"));
    }
}
