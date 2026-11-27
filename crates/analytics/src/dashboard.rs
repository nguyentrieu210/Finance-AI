//! Conservative dashboard projection from the public workflow read port.
use finance_workflow::analytics::StateCounts;
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Metric {
    pub name: String,
    pub value: String,
    pub unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denominator: Option<i64>,
    pub as_of: String,
}

/// Current-state counts are factual. This deliberately does not infer
/// first-pass acceptance or active handling time without immutable history.
pub fn from_state_counts(counts: StateCounts, as_of: &str) -> Vec<Metric> {
    [
        ("dossiers.total", counts.total),
        ("dossiers.draft", counts.draft),
        ("dossiers.processing", counts.processing),
        ("dossiers.needs_input", counts.needs_input),
        ("dossiers.reviewable", counts.reviewable),
        ("dossiers.ready", counts.ready),
        ("dossiers.exported", counts.exported),
        ("dossiers.handed_off", counts.handed_off),
        ("dossiers.accounting_review", counts.accounting_review),
        ("dossiers.accepted_current", counts.accepted),
        ("dossiers.returned_current", counts.returned),
        ("dossiers.cancelled", counts.cancelled),
    ]
    .into_iter()
    .map(|(name, value)| Metric {
        name: name.into(),
        value: value.to_string(),
        unit: "case".into(),
        denominator: None,
        as_of: as_of.into(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_first_pass_metric_without_history() {
        let metrics = from_state_counts(StateCounts { total: 3, accepted: 1, returned: 1, ..Default::default() }, "2026-11-27T00:00:00Z");
        assert!(metrics.iter().all(|m| !m.name.contains("first_pass")));
        assert_eq!(metrics.iter().find(|m| m.name=="dossiers.total").unwrap().value, "3");
    }
}
