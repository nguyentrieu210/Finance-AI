//! Bounded Tesseract TSV interchange for a synthetic independent OCR baseline.
//! No OCR process invocation, trusted source attestation or financial acceptance.
use crate::{Block, Error, PageResult};
use finance_documents::evidence::Rect;

const HEADER: &str =
    "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext";
const MAX_TSV_BYTES: usize = 2 * 1024 * 1024;
const MAX_ROWS: usize = 20_000;
const MAX_IMAGE_SIDE: u32 = 8192;
const SCALE: u64 = 1_000_000;

fn number(s: &str) -> Result<u32, Error> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::InvalidBlock);
    }
    s.parse().map_err(|_| Error::InvalidBlock)
}

// Exact decimal percent to basis points; reject overflow/negative/NaN rather
// than inventing a confidence. Tesseract's -1 means unavailable.
fn confidence(s: &str) -> Result<Option<u16>, Error> {
    if s == "-1" {
        return Ok(None);
    }
    let (whole, fraction) = s.split_once('.').unwrap_or((s, ""));
    let whole = number(whole)?;
    if whole > 100
        || fraction.len() > 6
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (whole == 100 && fraction.bytes().any(|b| b != b'0'))
    {
        return Err(Error::InvalidBlock);
    }
    let mut digits = fraction.bytes().chain(std::iter::repeat(b'0'));
    let bps = whole * 100
        + u32::from(digits.next().unwrap() - b'0') * 10
        + u32::from(digits.next().unwrap() - b'0');
    Ok(Some(bps as u16))
}

/// Input must be from one already oriented synthetic raster. Page numbering
/// refers to the original document, not the engine's per-image page number.
/// Pixel rectangle is converted outward to preserve all covered pixels.
pub fn parse_page(
    tsv: &str,
    page: u32,
    page_count: u32,
    width: u32,
    height: u32,
) -> Result<PageResult, Error> {
    if width == 0
        || height == 0
        || width > MAX_IMAGE_SIDE
        || height > MAX_IMAGE_SIDE
        || page == 0
        || page > page_count
        || page_count > super::MAX_PAGES
    {
        return Err(Error::InvalidPage);
    }
    if tsv.len() > MAX_TSV_BYTES {
        return Err(Error::TooMuchText);
    }
    let mut lines = tsv.lines();
    if lines.next() != Some(HEADER) {
        return Err(Error::InvalidBlock);
    }
    let mut blocks = Vec::new();
    let mut saw_page = false;
    for (index, line) in lines.enumerate() {
        if index >= MAX_ROWS {
            return Err(Error::TooMuchText);
        }
        let parts: Vec<_> = line.splitn(12, '\t').collect();
        if parts.len() != 12 {
            return Err(Error::InvalidBlock);
        }
        let level = number(parts[0])?;
        if number(parts[1])? != 1 || !(1..=5).contains(&level) {
            return Err(Error::InvalidBlock);
        }
        if level == 1 {
            if saw_page
                || index != 0
                || number(parts[6])? != 0
                || number(parts[7])? != 0
                || number(parts[8])? != width
                || number(parts[9])? != height
            {
                return Err(Error::InvalidPage);
            }
            saw_page = true;
        }
        if !saw_page {
            return Err(Error::InvalidPage);
        }
        if level != 5 {
            continue;
        }
        let x = number(parts[6])?;
        let y = number(parts[7])?;
        let w = number(parts[8])?;
        let h = number(parts[9])?;
        let right = x.checked_add(w).ok_or(Error::InvalidBlock)?;
        let bottom = y.checked_add(h).ok_or(Error::InvalidBlock)?;
        if w == 0 || h == 0 || right > width || bottom > height {
            return Err(Error::InvalidBlock);
        }
        if blocks.len() >= super::MAX_BLOCKS {
            return Err(Error::TooMuchText);
        }
        let floor = |n: u32, d: u32| (u64::from(n) * SCALE / u64::from(d)) as u32;
        let ceil = |n: u32, d: u32| (u64::from(n) * SCALE).div_ceil(u64::from(d)) as u32;
        let rect = Rect::new(
            floor(x, width),
            floor(y, height),
            ceil(right, width),
            ceil(bottom, height),
        )
        .map_err(|_| Error::InvalidBlock)?;
        blocks.push(Block::new(rect, parts[11], confidence(parts[10])?)?);
    }
    if !saw_page {
        return Err(Error::InvalidPage);
    }
    PageResult::new(page, page_count, blocks, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_bounds_and_confidence() {
        let input = format!(
            "{HEADER}\n1\t1\t0\t0\t0\t0\t0\t0\t100\t200\t-1\t\n5\t1\t1\t1\t1\t1\t10\t20\t30\t40\t98.765432\tTổng"
        );
        let page = parse_page(&input, 2, 3, 100, 200).unwrap();
        assert_eq!(page.page(), 2);
        assert_eq!(page.blocks()[0].text(), "Tổng");
        assert_eq!(page.blocks()[0].confidence(), Some(9876));
        assert_eq!(
            page.blocks()[0].rect().coordinates(),
            [100_000, 100_000, 400_000, 300_000]
        );
        assert!(parse_page(&input, 1, 1, 99, 200).is_err());
        assert!(parse_page(&input.replace("98.765432", "100.1"), 1, 1, 100, 200).is_err());
        assert!(parse_page(&input.replace("30\t40", "100\t40"), 1, 1, 100, 200).is_err());
        assert!(parse_page(&input.replace("Tổng", "bad\0text"), 1, 1, 100, 200).is_err());
        assert!(parse_page(&input, 1, 201, 100, 200).is_err());
    }
}
