use finance_document_intelligence::{
    Kind,
    normalize::{Format, Outcome},
};
use finance_extraction::{
    Field, State,
    rows::{OwnedValue, extract},
};
use finance_ocr::tsv::parse_page;
use finance_shared::money::Amount;
use std::fs;

#[test]
#[ignore = "requires actual Vietnamese OCR fixture"]
fn real_word_rows_to_financial_candidates() {
    let input = fs::read_to_string(std::env::var("FINANCE_OCR_ROWS_TSV").unwrap()).unwrap();
    let page = parse_page(&input, 1, 1, 1800, 480).unwrap();
    let fields = extract(Kind::Invoice, std::slice::from_ref(&page), Format::Vietnamese).unwrap();
    let total = fields.iter().find(|f| f.field == Field::TotalAmount).unwrap();
    assert_eq!(total.state, State::Candidate);
    assert_eq!(total.observations.len(), 1);
    let row = &total.observations[0];
    assert!(row.outcome == Outcome::Candidate(OwnedValue::Amount(Amount::parse("125000").unwrap())));
    assert!(row.block_indexes.len() >= 2);
    assert_eq!(
        row.reconstructed_row,
        row.block_indexes
            .iter()
            .map(|&i| page.blocks()[i].text())
            .collect::<Vec<_>>()
            .join(" ")
    );
    let tax = fields.iter().find(|f| f.field == Field::TaxAmount).unwrap();
    assert_eq!(tax.state, State::Unknown);
}
