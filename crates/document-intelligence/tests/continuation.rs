use finance_document_intelligence::table::{self, CellProposal, Continuation, Slot};
use finance_documents::evidence::Rect;
use finance_ocr::{Block, PageResult};

fn page(index: u32, header: &str) -> PageResult {
    let block = |text: &str, bounds: [u32; 4]| {
        let [x0, y0, x1, y1] = bounds;
        Block::new(Rect::new(x0, y0, x1, y1).unwrap(), text, None).unwrap()
    };
    PageResult::new(
        index,
        3,
        vec![
            block("Nội dung", [10_000, 10_000, 300_000, 200_000]),
            block(header, [510_000, 10_000, 900_000, 200_000]),
            block("125000", [510_000, 600_000, 900_000, 700_000]),
        ],
        None,
    )
    .unwrap()
}

fn grid(page: &PageResult) -> table::Table {
    let edges = [0, 500_000, 1_000_000];
    let cells = [(0, 0, 0), (0, 1, 1), (1, 1, 2)]
        .into_iter()
        .map(|(row, column, block)| CellProposal {
            row,
            column,
            row_span: 1,
            column_span: 1,
            block_indexes: vec![block],
        })
        .collect();
    table::validate(page, &edges, &edges, cells).unwrap()
}

#[test]
fn repeated_headers_are_candidates_not_merged_data() {
    let first = page(1, "Số tiền");
    let second = page(2, "Số tiền");
    let a = grid(&first);
    let b = grid(&second);
    assert_eq!(table::continuation(&a, &b), Continuation::Candidate);
    assert_eq!(b.slot(1, 0), Some(Slot::Unknown));
    assert_eq!(b.page(), 2);
}

#[test]
fn mismatched_or_nonadjacent_pages_are_not_joined() {
    let first = grid(&page(1, "Số tiền"));
    let wrong = grid(&page(2, "Số lượng"));
    let gap = grid(&page(3, "Số tiền"));
    assert_eq!(table::continuation(&first, &wrong), Continuation::Unknown);
    assert_eq!(table::continuation(&first, &gap), Continuation::Unknown);
}
