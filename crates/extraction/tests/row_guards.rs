use finance_document_intelligence::{Kind, normalize::Format};
use finance_documents::evidence::Rect;
use finance_extraction::{Field, State, rows::extract};
use finance_ocr::{Block, PageResult};

fn block(text: &str, x0: u32, x1: u32, y: u32) -> Block {
    Block::new(Rect::new(x0, y, x1, y + 20_000).unwrap(), text, Some(9999)).unwrap()
}

#[test]
fn unrelated_columns_and_duplicate_candidates_do_not_verify() {
    let page = PageResult::new(
        1,
        1,
        vec![
            block("Tổng", 10_000, 80_000, 100_000),
            block("tiền:", 90_000, 150_000, 100_000),
            block("125000", 600_000, 700_000, 100_000),
        ],
        None,
    )
    .unwrap();
    let fields = extract(Kind::Invoice, &[page], Format::Vietnamese).unwrap();
    assert_eq!(
        fields
            .iter()
            .find(|f| f.field == Field::TotalAmount)
            .unwrap()
            .state,
        State::Unknown
    );
    let page = PageResult::new(
        1,
        1,
        vec![
            block("Tổng tiền:", 10_000, 150_000, 100_000),
            block("125000", 160_000, 260_000, 100_000),
            block("Tổng tiền:", 10_000, 150_000, 200_000),
            block("126000", 160_000, 260_000, 200_000),
        ],
        None,
    )
    .unwrap();
    let fields = extract(Kind::Invoice, &[page], Format::Vietnamese).unwrap();
    let total = fields
        .iter()
        .find(|f| f.field == Field::TotalAmount)
        .unwrap();
    assert_eq!(total.state, State::Ambiguous);
    assert_eq!(total.observations.len(), 2);
}
