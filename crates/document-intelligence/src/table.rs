//! Validate an externally proposed grid against OCR page geometry.
//! This is a table candidate, not table detection or financial extraction.
use finance_ocr::PageResult;

const MAX_ROWS: usize = 200;
const MAX_COLUMNS: usize = 64;
const MAX_SLOTS: usize = 10_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidGrid,
    InvalidCell,
    OverlappingCell,
    DuplicateBlock,
    BlockOutsideCell,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CellProposal {
    /// Zero-based inclusive start, positive span.
    pub row: usize,
    pub column: usize,
    pub row_span: usize,
    pub column_span: usize,
    pub block_indexes: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Slot {
    /// No supplied cell, or supplied cell with no recognized content.
    Unknown,
    /// A source-backed anchor cell, or another slot spanned by that cell.
    Anchor(usize),
    Covered(usize),
}

pub struct Table {
    page: u32,
    rows: usize,
    columns: usize,
    cells: Vec<CellProposal>,
    slots: Vec<Slot>,
}

impl Table {
    pub fn page(&self) -> u32 {
        self.page
    }

    pub fn dimensions(&self) -> (usize, usize) {
        (self.rows, self.columns)
    }

    pub fn cells(&self) -> &[CellProposal] {
        &self.cells
    }

    pub fn slot(&self, row: usize, column: usize) -> Option<Slot> {
        if row >= self.rows || column >= self.columns {
            return None;
        }
        Some(self.slots[row * self.columns + column])
    }

    pub fn requires_review(&self) -> bool {
        true
    }
}

fn valid_tracks(tracks: &[u32], maximum: usize) -> bool {
    tracks.len() >= 2
        && tracks.len() <= maximum + 1
        && tracks.last().is_some_and(|&v| v <= 1_000_000)
        && tracks.windows(2).all(|w| w[0] < w[1])
}

/// Tracks are normalized source-page edges. A grid proposer must provide them;
/// this function does not infer missing tracks or cell values. Overlapping
/// spans, reused source blocks, and out-of-cell source references are rejected.
pub fn validate(
    page: &PageResult,
    row_edges: &[u32],
    column_edges: &[u32],
    cells: Vec<CellProposal>,
) -> Result<Table, Error> {
    if !valid_tracks(row_edges, MAX_ROWS) || !valid_tracks(column_edges, MAX_COLUMNS) {
        return Err(Error::InvalidGrid);
    }
    let rows = row_edges.len() - 1;
    let columns = column_edges.len() - 1;
    if rows * columns > MAX_SLOTS || cells.len() > rows * columns {
        return Err(Error::InvalidGrid);
    }
    let mut occupied = vec![false; rows * columns];
    let mut slots = vec![Slot::Unknown; rows * columns];
    let mut used_blocks = vec![false; page.blocks().len()];
    for (index, cell) in cells.iter().enumerate() {
        let row_end = cell
            .row
            .checked_add(cell.row_span)
            .ok_or(Error::InvalidCell)?;
        let column_end = cell
            .column
            .checked_add(cell.column_span)
            .ok_or(Error::InvalidCell)?;
        if cell.row_span == 0
            || cell.column_span == 0
            || row_end > rows
            || column_end > columns
            || cell.block_indexes.len() > page.blocks().len()
        {
            return Err(Error::InvalidCell);
        }
        for &i in &cell.block_indexes {
            let block = page.blocks().get(i).ok_or(Error::InvalidCell)?;
            if used_blocks[i] {
                return Err(Error::DuplicateBlock);
            }
            let [x0, y0, x1, y1] = block.rect().coordinates();
            if x0 < column_edges[cell.column]
                || x1 > column_edges[column_end]
                || y0 < row_edges[cell.row]
                || y1 > row_edges[row_end]
            {
                return Err(Error::BlockOutsideCell);
            }
            used_blocks[i] = true;
        }
        for row in cell.row..row_end {
            for column in cell.column..column_end {
                let offset = row * columns + column;
                if occupied[offset] {
                    return Err(Error::OverlappingCell);
                }
                occupied[offset] = true;
                if !cell.block_indexes.is_empty() {
                    slots[offset] = if row == cell.row && column == cell.column {
                        Slot::Anchor(index)
                    } else {
                        Slot::Covered(index)
                    };
                }
            }
        }
    }
    Ok(Table {
        page: page.page(),
        rows,
        columns,
        cells,
        slots,
    })
}
