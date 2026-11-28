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
        let metrics = from_state_counts(
            StateCounts {
                total: 3,
                accepted: 1,
                returned: 1,
                ..Default::default()
            },
            "2026-11-27T00:00:00Z",
        );
        assert!(metrics.iter().all(|m| !m.name.contains("first_pass")));
        assert_eq!(
            metrics
                .iter()
                .find(|m| m.name == "dossiers.total")
                .unwrap()
                .value,
            "3"
        );
    }
}

pub fn from_review_history(
    counts: finance_workflow::analytics::ReviewHistoryCounts,
    as_of: &str,
) -> Vec<Metric> {
    let mut metrics = Vec::new();
    for (name, value) in [
        ("reviews.accepted_verified", counts.accepted),
        ("reviews.returned_cases_recorded", counts.returned_cases),
        ("reviews.accepted_history_unknown", counts.unknown_accepted),
    ] {
        metrics.push(Metric {
            name: name.into(),
            value: value.to_string(),
            unit: "case".into(),
            denominator: None,
            as_of: as_of.into(),
        });
    }
    if counts.accepted > 0 && counts.unknown_accepted == 0 {
        metrics.push(Metric {
            name: "reviews.first_pass_accepted".into(),
            value: counts.first_pass.to_string(),
            unit: "case".into(),
            denominator: Some(counts.accepted),
            as_of: as_of.into(),
        });
    }
    metrics
}

#[cfg(test)]
mod history_tests {
    use super::*;
    use finance_workflow::analytics::ReviewHistoryCounts;

    #[test]
    fn first_pass_is_omitted_when_accepted_history_is_unknown() {
        let counts = ReviewHistoryCounts {
            accepted: 3,
            first_pass: 2,
            returned_cases: 1,
            unknown_accepted: 1,
        };
        let partial = from_review_history(counts, "2026-11-28T00:00:00Z");
        assert!(!partial.iter().any(|metric| metric.name == "reviews.first_pass_accepted"));
        let complete = from_review_history(
            ReviewHistoryCounts { unknown_accepted: 0, ..counts },
            "2026-11-28T00:00:00Z",
        );
        let metric = complete.iter().find(|m| m.name == "reviews.first_pass_accepted").unwrap();
        assert_eq!(metric.value, "2");
        assert_eq!(metric.denominator, Some(3));
    }
}
