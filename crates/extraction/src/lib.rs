#![forbid(unsafe_code)]
//! E08 deterministic synthetic candidates only. No provider, verification,
//! database writes, financial approval or automatic workflow routing.
use chrono::NaiveDate;
use finance_document_intelligence::{
    Kind,
    normalize::{self, Format, Outcome},
};
use finance_ocr::PageResult;
use finance_shared::money::Amount;
use unicode_normalization::UnicodeNormalization;

pub mod gateway;
pub mod repository;
pub mod rows;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Field {
    ContractNumber,
    InvoiceNumber,
    DocumentDate,
    Supplier,
    AmountBeforeTax,
    TaxAmount,
    TotalAmount,
    RequestedAmount,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Value<'a> {
    Identifier(&'a str),
    Text(&'a str),
    Date(NaiveDate),
    Amount(Amount),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Unknown,
    Invalid,
    Ambiguous,
    Candidate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidPages,
    TooManyCandidates,
}

/// Raw value is an exact byte slice from an original OCR block. The locator
/// references that block, not a fabricated substring bounding rectangle.
/// PageResult itself is an unverified candidate; immutable source and revision
/// attestation must be checked by the future repository.
pub struct Observation<'a> {
    pub page: u32,
    pub block_index: usize,
    pub value_start: usize,
    pub value_end: usize,
    pub raw: &'a str,
    pub outcome: Outcome<Value<'a>>,
    pub confidence_bps: Option<u16>,
}

pub struct FieldResult<'a> {
    pub field: Field,
    pub state: State,
    pub observations: Vec<Observation<'a>>,
}

impl FieldResult<'_> {
    pub fn requires_review(&self) -> bool {
        true
    }
}

fn fields(kind: Kind) -> &'static [Field] {
    use Field::*;
    match kind {
        Kind::Contract | Kind::ContractAppendix => {
            &[ContractNumber, DocumentDate, Supplier, TotalAmount]
        }
        Kind::Invoice => &[
            InvoiceNumber,
            DocumentDate,
            Supplier,
            AmountBeforeTax,
            TaxAmount,
            TotalAmount,
        ],
        Kind::PaymentRequest => &[
            ContractNumber,
            InvoiceNumber,
            DocumentDate,
            Supplier,
            RequestedAmount,
        ],
        Kind::AdvanceRequest | Kind::AdvanceSettlement => {
            &[ContractNumber, DocumentDate, RequestedAmount]
        }
        Kind::AcceptanceRecord => &[ContractNumber, DocumentDate, TotalAmount],
        Kind::PaymentVoucher => &[DocumentDate, RequestedAmount],
    }
}

fn label(raw: &str) -> Option<Field> {
    let key = raw.nfc().collect::<String>().to_lowercase();
    let key = key.split_whitespace().collect::<Vec<_>>().join(" ");
    match key.as_str() {
        "số hợp đồng" => Some(Field::ContractNumber),
        "số hóa đơn" => Some(Field::InvoiceNumber),
        "ngày chứng từ" | "ngày lập" | "ngày hóa đơn" => Some(Field::DocumentDate),
        "nhà cung cấp" | "tên nhà cung cấp" => Some(Field::Supplier),
        "tiền trước thuế" | "cộng tiền hàng" => Some(Field::AmountBeforeTax),
        "tiền thuế" | "tiền thuế gtgt" => Some(Field::TaxAmount),
        "tổng tiền" | "tổng thanh toán" => Some(Field::TotalAmount),
        "số tiền đề nghị" | "số tiền đề nghị thanh toán" | "số tiền đề nghị tạm ứng" => {
            Some(Field::RequestedAmount)
        }
        _ => None,
    }
}

fn map_outcome<T, U>(outcome: Outcome<T>, map: impl FnOnce(T) -> U) -> Outcome<U> {
    match outcome {
        Outcome::Unknown => Outcome::Unknown,
        Outcome::Invalid => Outcome::Invalid,
        Outcome::Ambiguous => Outcome::Ambiguous,
        Outcome::Candidate(value) => Outcome::Candidate(map(value)),
    }
}

fn parse(field: Field, raw: &str, format: Format) -> Outcome<Value<'_>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Outcome::Unknown;
    }
    match field {
        Field::ContractNumber | Field::InvoiceNumber => {
            if trimmed.len() <= 128
                && trimmed
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
            {
                Outcome::Candidate(Value::Identifier(trimmed))
            } else {
                Outcome::Invalid
            }
        }
        Field::Supplier => {
            if trimmed.len() <= 512 && !trimmed.chars().any(char::is_control) {
                Outcome::Candidate(Value::Text(trimmed))
            } else {
                Outcome::Invalid
            }
        }
        Field::DocumentDate => map_outcome(normalize::date(raw, format).outcome, Value::Date),
        _ => map_outcome(normalize::amount(raw, format).outcome, Value::Amount),
    }
}

fn summarize(observations: &[Observation<'_>]) -> State {
    let mut value = None;
    let mut invalid = false;
    let mut unknown = false;
    for item in observations {
        match item.outcome {
            Outcome::Ambiguous => return State::Ambiguous,
            Outcome::Invalid => invalid = true,
            Outcome::Unknown => unknown = true,
            Outcome::Candidate(current) => {
                if value.is_some_and(|previous| previous != current) {
                    return State::Ambiguous;
                }
                value = Some(current);
            }
        }
    }
    match (value.is_some(), invalid, unknown) {
        (true, true, _) | (true, _, true) => State::Ambiguous,
        (true, false, false) => State::Candidate,
        (false, true, _) => State::Invalid,
        (false, false, _) => State::Unknown,
    }
}

/// Accept one already selected document kind and consecutive source pages.
/// Exact label:value blocks only. No fuzzy inference, cross-row pairing, LLM
/// or implicit mapping from an invoice total to a requested payment.
pub fn extract(
    kind: Kind,
    pages: &[PageResult],
    format: Format,
) -> Result<Vec<FieldResult<'_>>, Error> {
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
        for (block_index, block) in page.blocks().iter().enumerate() {
            let text = block.text();
            let Some((name, raw)) = text.split_once(':') else {
                continue;
            };
            if name.len() > 128 || name.contains(['\n', '\t']) {
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
            result.observations.push(Observation {
                page: page.page(),
                block_index,
                value_start: name.len() + 1,
                value_end: text.len(),
                raw,
                outcome: parse(field, raw, format),
                confidence_bps: block.confidence(),
            });
        }
    }
    for result in &mut results {
        result.state = summarize(&result.observations);
    }
    Ok(results)
}
