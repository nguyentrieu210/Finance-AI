//! Source-backed candidate extraction from contiguous OCR word blocks.
//! A reconstructed row is not original source text or verified evidence.
use super::{Error, Field, State, Value, fields, label, map_outcome, parse};
use finance_document_intelligence::{
    Kind, layout,
    normalize::{Format, Outcome},
};
use finance_ocr::PageResult;

#[derive(Clone, Eq, PartialEq)]
pub enum OwnedValue {
    Identifier(String),
    Text(String),
    Date(chrono::NaiveDate),
    Amount(finance_shared::money::Amount),
}

pub struct Observation {
    pub page: u32,
    pub block_indexes: Vec<usize>,
    pub reconstructed_row: String,
    pub outcome: Outcome<OwnedValue>,
}

pub struct FieldResult {
    pub field: Field,
    pub state: State,
    pub observations: Vec<Observation>,
}

fn own(value: Value<'_>) -> OwnedValue {
    match value {
        Value::Identifier(s) => OwnedValue::Identifier(s.to_owned()),
        Value::Text(s) => OwnedValue::Text(s.to_owned()),
        Value::Date(d) => OwnedValue::Date(d),
        Value::Amount(a) => OwnedValue::Amount(a),
    }
}

fn summarize(observations: &[Observation]) -> State {
    let mut known = None;
    let mut invalid = false;
    let mut unknown = false;
    for item in observations {
        match &item.outcome {
            Outcome::Ambiguous => return State::Ambiguous,
            Outcome::Invalid => invalid = true,
            Outcome::Unknown => unknown = true,
            Outcome::Candidate(value) => {
                if known.is_some_and(|previous| previous != value) {
                    return State::Ambiguous;
                }
                known = Some(value);
            }
        }
    }
    match (known.is_some(), invalid, unknown) {
        (true, true, _) | (true, _, true) => State::Ambiguous,
        (true, false, false) => State::Candidate,
        (false, true, _) => State::Invalid,
        (false, false, _) => State::Unknown,
    }
}

/// Only short, contiguous, nonoverlapping row candidates are accepted.
/// Reject entire row at a gap >3.5% of page width instead of borrowing a
/// number from an unrelated column. Every source block remains accessible.
pub fn extract(
    kind: Kind,
    pages: &[PageResult],
    format: Format,
) -> Result<Vec<FieldResult>, Error> {
    if pages.is_empty()
        || pages.len() > 200
        || pages
            .windows(2)
            .any(|w| w[0].page().checked_add(1) != Some(w[1].page()))
    {
        return Err(Error::InvalidPages);
    }
    let mut results: Vec<_> = fields(kind)
        .iter()
        .map(|&field| FieldResult {
            field,
            state: State::Unknown,
            observations: Vec::new(),
        })
        .collect();
    for page in pages {
        let analysis = layout::analyze(page);
        for row in analysis.rows {
            let indexes = row.block_indexes;
            if indexes.is_empty() || indexes.len() > 16 {
                continue;
            }
            let blocks = page.blocks();
            if indexes.windows(2).any(|w| {
                let left = blocks[w[0]].rect().coordinates();
                let right = blocks[w[1]].rect().coordinates();
                right[0] < left[2] || right[0].saturating_sub(left[2]) > 35_000
            }) {
                continue;
            }
            let text = indexes
                .iter()
                .map(|&i| blocks[i].text())
                .collect::<Vec<_>>()
                .join(" ");
            if text.len() > 4096 || text.contains(['\n', '\t']) {
                continue;
            }
            let Some((name, raw)) = text.split_once(':') else {
                continue;
            };
            if name.len() > 128 || raw.contains(':') {
                continue;
            }
            let Some(field) = label(name) else {
                continue;
            };
            let Some(result) = results.iter_mut().find(|r| r.field == field) else {
                continue;
            };
            if result.observations.len() >= 64 {
                return Err(Error::TooManyCandidates);
            }
            let outcome = map_outcome(parse(field, raw, format), own);
            result.observations.push(Observation {
                page: page.page(),
                block_indexes: indexes,
                reconstructed_row: text,
                outcome,
            });
        }
    }
    for result in &mut results {
        result.state = summarize(&result.observations);
    }
    Ok(results)
}
