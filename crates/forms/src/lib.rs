#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
use std::collections::{BTreeMap, BTreeSet};

pub struct Field<'a> {
    pub key: &'a str,
    pub required: bool,
    pub depends_on: Option<&'a str>,
}

/// Validate a bounded dependency graph. No executable conditions.
pub fn validate(fields: &[Field<'_>]) -> bool {
    if fields.is_empty() || fields.len() > 256 {
        return false;
    }
    let mut by_key = BTreeMap::new();
    for field in fields {
        if field.key.is_empty() || field.key.len() > 64 || !field.key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') || by_key.insert(field.key, field).is_some() {
            return false;
        }
    }
    for field in fields {
        let mut seen = BTreeSet::new();
        let mut current = Some(field.key);
        while let Some(key) = current {
            if !seen.insert(key) {
                return false;
            }
            let Some(item) = by_key.get(key) else {
                return false;
            };
            current = item.depends_on;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_cycles_and_missing_dependencies() {
        assert!(!validate(&[Field { key: "a", required: true, depends_on: Some("a") }]));
        assert!(!validate(&[Field { key: "a", required: false, depends_on: Some("b") }]));
        assert!(validate(&[Field { key: "a", required: true, depends_on: None }]));
    }
}
