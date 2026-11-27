#![forbid(unsafe_code)]
//! Synthetic-only domain prototype. No authorization, durable mutation or production claim.
use finance_shared::ids::{CompanyScope, DossierId};
use std::collections::BTreeMap;

pub struct Case {
    pub scope: CompanyScope,
    pub dossier: DossierId,
    pub accepted: bool,
    pub returned: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Counts {
    pub cases: usize,
    pub accepted: usize,
    pub first_pass: usize,
}

/// Scoped deduplication of synthetic case observations. Any recorded return
/// excludes first-pass acceptance. No duration or cohort inference.
pub fn counts(scope: CompanyScope, rows: &[Case]) -> Counts {
    let mut cases = BTreeMap::<DossierId, (bool, bool)>::new();
    for row in rows.iter().filter(|r| r.scope == scope) {
        let entry = cases.entry(row.dossier).or_default();
        entry.0 |= row.accepted;
        entry.1 |= row.returned;
    }
    Counts {
        cases: cases.len(),
        accepted: cases.values().filter(|(accepted, _)| *accepted).count(),
        first_pass: cases
            .values()
            .filter(|(accepted, returned)| *accepted && !*returned)
            .count(),
    }
}

/// CSV export defensive cell quoting. Prefix dangerous leading characters
/// after whitespace as well; authorization is outside this encoder.
pub fn csv_cell(value: &str) -> String {
    let dangerous = value
        .trim_start_matches(|c: char| c.is_whitespace())
        .starts_with(['=', '+', '-', '@'])
        || value.starts_with(['\t', '\r', '\n']);
    let prefix = if dangerous { "'" } else { "" };
    format!("\"{}{}\"", prefix, value.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_formula_and_quote_escaped() {
        assert_eq!(csv_cell("=1+1"), "\"'=1+1\"");
        assert_eq!(csv_cell("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_cell("  @x"), "\"'  @x\"");
    }
}
