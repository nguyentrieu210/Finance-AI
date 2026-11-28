//! Conservative row-major layout candidates, never a verified reading order.
use finance_ocr::PageResult;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Warning {
    WideGap,
    OverlappingBlocks,
    MultipleRows,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub block_indexes: Vec<usize>,
    pub bounds: [u32; 4],
}

pub struct Layout {
    pub page: u32,
    pub rows: Vec<Row>,
    pub warnings: Vec<Warning>,
}

impl Layout {
    pub fn requires_review(&self) -> bool {
        true
    }
}

/// A deterministic row-major proposal, preserving original block indexes.
/// Use the first rectangle as row anchor to avoid transitive overlap chains.
/// Wide gaps or overlapping blocks may indicate multiple columns, tables or
/// duplicate text layers. They never establish a single trusted reading order.
pub fn analyze(page: &PageResult) -> Layout {
    let mut order: Vec<_> = (0..page.blocks().len()).collect();
    let rect = |i: usize| page.blocks()[i].rect().coordinates();
    order.sort_by_key(|&i| {
        let [x, y, _, _] = rect(i);
        (y, x, i)
    });
    let mut rows: Vec<Row> = Vec::new();
    let mut warnings = Vec::new();
    for i in order {
        let [x0, y0, x1, y1] = rect(i);
        if let Some(row) = rows.last_mut() {
            let [_, top, _, bottom] = rect(row.block_indexes[0]);
            let overlap = bottom.min(y1).saturating_sub(top.max(y0));
            let height = (bottom - top).max(y1 - y0);
            if overlap * 2 >= height {
                row.block_indexes.push(i);
                row.bounds = [
                    row.bounds[0].min(x0),
                    row.bounds[1].min(y0),
                    row.bounds[2].max(x1),
                    row.bounds[3].max(y1),
                ];
                continue;
            }
            if y0 < row.bounds[3] && !warnings.contains(&Warning::MultipleRows) {
                warnings.push(Warning::MultipleRows);
            }
        }
        rows.push(Row {
            block_indexes: vec![i],
            bounds: [x0, y0, x1, y1],
        });
    }
    for row in &mut rows {
        row.block_indexes.sort_by_key(|&i| (rect(i)[0], i));
        let mut right = 0;
        for (position, &i) in row.block_indexes.iter().enumerate() {
            let [x0, _, x1, _] = rect(i);
            if position > 0 {
                if x0 < right && !warnings.contains(&Warning::OverlappingBlocks) {
                    warnings.push(Warning::OverlappingBlocks);
                }
                if x0.saturating_sub(right) > 80_000 && !warnings.contains(&Warning::WideGap) {
                    warnings.push(Warning::WideGap);
                }
            }
            right = right.max(x1);
        }
    }
    Layout {
        page: page.page(),
        rows,
        warnings,
    }
}
