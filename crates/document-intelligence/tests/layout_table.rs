use finance_document_intelligence::{layout, table};
use finance_documents::evidence::Rect;
use finance_ocr::{Block, PageResult};

fn block(text: &str, bounds: [u32; 4]) -> Block {
    let [x0, y0, x1, y1] = bounds;
    Block::new(Rect::new(x0, y0, x1, y1).unwrap(), text, None).unwrap()
}

#[test]
fn layout_preserves_indexes_and_flags_columns() {
    let page = PageResult::new(
        2,
        2,
        vec![
            block("125000", [600_000, 100_000, 800_000, 130_000]),
            block("Tổng tiền", [50_000, 100_000, 200_000, 130_000]),
            block("VNĐ", [50_000, 200_000, 120_000, 230_000]),
        ],
        None,
    )
    .unwrap();
    let result = layout::analyze(&page);
    assert_eq!(result.page, 2);
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0].block_indexes, vec![1, 0]);
    assert_eq!(result.rows[1].block_indexes, vec![2]);
    assert!(result.warnings.contains(&layout::Warning::WideGap));
    assert!(result.requires_review());
    assert_eq!(page.blocks()[0].text(), "125000");
}

#[test]
fn overlapping_blocks_are_not_deduplicated() {
    let page = PageResult::new(
        1,
        1,
        vec![
            block("A", [100, 100, 300, 200]),
            block("B", [200, 100, 400, 200]),
        ],
        None,
    )
    .unwrap();
    let result = layout::analyze(&page);
    assert_eq!(result.rows[0].block_indexes, vec![0, 1]);
    assert!(
        result
            .warnings
            .contains(&layout::Warning::OverlappingBlocks)
    );
}

fn grid_page() -> PageResult {
    PageResult::new(
        1,
        1,
        vec![
            block("Bảng thanh toán", [100_000, 100_000, 900_000, 200_000]),
            block("125000", [600_000, 600_000, 900_000, 700_000]),
        ],
        None,
    )
    .unwrap()
}

fn cell(row: usize, column: usize, span: usize, blocks: Vec<usize>) -> table::CellProposal {
    table::CellProposal {
        row,
        column,
        row_span: 1,
        column_span: span,
        block_indexes: blocks,
    }
}

#[test]
fn merged_and_missing_cells_remain_source_backed_or_unknown() {
    let edges = [0, 500_000, 1_000_000];
    let grid = table::validate(
        &grid_page(),
        &edges,
        &edges,
        vec![cell(0, 0, 2, vec![0]), cell(1, 1, 1, vec![1])],
    )
    .unwrap();
    assert_eq!(grid.dimensions(), (2, 2));
    assert_eq!(grid.page(), 1);
    assert_eq!(grid.slot(0, 0), Some(table::Slot::Anchor(0)));
    assert_eq!(grid.slot(0, 1), Some(table::Slot::Covered(0)));
    assert_eq!(grid.slot(1, 0), Some(table::Slot::Unknown));
    assert_eq!(grid.slot(1, 1), Some(table::Slot::Anchor(1)));
    assert_eq!(grid.slot(2, 0), None);
    assert!(grid.requires_review());
}

#[test]
fn invalid_spans_and_source_reuse_are_rejected() {
    let page = grid_page();
    let edges = [0, 500_000, 1_000_000];
    let overlapping = vec![cell(0, 0, 2, vec![]), cell(0, 1, 1, vec![])];
    assert_eq!(
        table::validate(&page, &edges, &edges, overlapping).err(),
        Some(table::Error::OverlappingCell)
    );
    let reused = vec![cell(0, 0, 2, vec![0, 0])];
    assert_eq!(
        table::validate(&page, &edges, &edges, reused).err(),
        Some(table::Error::DuplicateBlock)
    );
    let outside = vec![cell(0, 0, 1, vec![1])];
    assert_eq!(
        table::validate(&page, &edges, &edges, outside).err(),
        Some(table::Error::BlockOutsideCell)
    );
    let overflow = vec![cell(0, 1, usize::MAX, vec![])];
    assert_eq!(
        table::validate(&page, &edges, &edges, overflow).err(),
        Some(table::Error::InvalidCell)
    );
    assert!(table::validate(&page, &[0, 0], &edges, vec![]).is_err());
}
