//! Synthetic PaddleOCR interchange contract, not an inference runtime.
use crate::{Block, Error, PageResult};
use finance_documents::evidence::{Rect, Rotation};

const MAX_SIDE: u32 = 8192;
const SCALE: u64 = 1_000_000;

/// Pixel-space quadrilateral from an already display-oriented raster.
/// Coordinates are integer pixel edges. Provider confidence is in basis points.
pub struct Detection<'a> {
    pub points: [[u32; 2]; 4],
    pub text: &'a str,
    pub confidence_bps: Option<u16>,
}

/// Explicit orientation and dimensions supplied by a trusted renderer.
/// No provider-side automatic page rotation may silently change this frame.
#[derive(Clone, Copy)]
pub struct Frame {
    pub page: u32,
    pub page_count: u32,
    pub width: u32,
    pub height: u32,
    pub rotation: Rotation,
}

fn cross(a: [u32; 2], b: [u32; 2], c: [u32; 2]) -> i64 {
    let dx = i64::from(b[0]) - i64::from(a[0]);
    let dy = i64::from(b[1]) - i64::from(a[1]);
    let ex = i64::from(c[0]) - i64::from(b[0]);
    let ey = i64::from(c[1]) - i64::from(b[1]);
    dx * ey - dy * ex
}

fn rectangle(points: [[u32; 2]; 4], frame: Frame) -> Result<Rect, Error> {
    if points
        .iter()
        .any(|p| p[0] > frame.width || p[1] > frame.height)
    {
        return Err(Error::InvalidBlock);
    }
    // A convex, consistently ordered, nondegenerate polygon is required.
    let turns = [
        cross(points[0], points[1], points[2]),
        cross(points[1], points[2], points[3]),
        cross(points[2], points[3], points[0]),
        cross(points[3], points[0], points[1]),
    ];
    if !turns.iter().all(|&n| n > 0) && !turns.iter().all(|&n| n < 0) {
        return Err(Error::InvalidBlock);
    }
    let x0 = points.iter().map(|p| p[0]).min().unwrap();
    let y0 = points.iter().map(|p| p[1]).min().unwrap();
    let x1 = points.iter().map(|p| p[0]).max().unwrap();
    let y1 = points.iter().map(|p| p[1]).max().unwrap();
    let floor = |n: u32, d: u32| (u64::from(n) * SCALE / u64::from(d)) as u32;
    let ceil = |n: u32, d: u32| (u64::from(n) * SCALE).div_ceil(u64::from(d)) as u32;
    let display = Rect::new(
        floor(x0, frame.width),
        floor(y0, frame.height),
        ceil(x1, frame.width),
        ceil(y1, frame.height),
    )
    .map_err(|_| Error::InvalidBlock)?;
    let inverse = match frame.rotation {
        Rotation::R0 => Rotation::R0,
        Rotation::R90 => Rotation::R270,
        Rotation::R180 => Rotation::R180,
        Rotation::R270 => Rotation::R90,
    };
    Ok(display.rotate(inverse))
}

/// Convert bounded typed detections to unrotated original-page rectangles.
/// Retains candidate text, never treats confidence as verification.
pub fn convert_page(frame: Frame, detections: &[Detection<'_>]) -> Result<PageResult, Error> {
    if frame.width == 0
        || frame.height == 0
        || frame.width > MAX_SIDE
        || frame.height > MAX_SIDE
        || frame.page == 0
        || frame.page > frame.page_count
        || frame.page_count > super::MAX_PAGES
    {
        return Err(Error::InvalidPage);
    }
    if detections.len() > super::MAX_BLOCKS {
        return Err(Error::TooMuchText);
    }
    let mut total = 0_usize;
    let mut blocks = Vec::with_capacity(detections.len());
    for item in detections {
        total = total
            .checked_add(item.text.len())
            .ok_or(Error::TooMuchText)?;
        if total > super::MAX_PAGE_TEXT_BYTES {
            return Err(Error::TooMuchText);
        }
        blocks.push(Block::new(
            rectangle(item.points, frame)?,
            item.text,
            item.confidence_bps,
        )?);
    }
    PageResult::new(frame.page, frame.page_count, blocks, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ReviewState;

    fn frame(rotation: Rotation) -> Frame {
        Frame {
            page: 2,
            page_count: 3,
            width: 200,
            height: 100,
            rotation,
        }
    }

    fn detection(points: [[u32; 2]; 4]) -> Detection<'static> {
        Detection {
            points,
            text: "Tổng thanh toán: 125000",
            confidence_bps: Some(9999),
        }
    }

    #[test]
    fn original_coordinates_and_review() {
        let item = detection([[20, 10], [80, 10], [80, 30], [20, 30]]);
        let page = convert_page(frame(Rotation::R90), &[item]).unwrap();
        assert_eq!(page.page(), 2);
        assert_eq!(
            page.blocks()[0].rect().coordinates(),
            [100_000, 600_000, 300_000, 900_000]
        );
        assert_eq!(page.blocks()[0].text(), "Tổng thanh toán: 125000");
        assert_eq!(page.review_state(), ReviewState::NeedsHumanReview);
    }

    #[test]
    fn invalid_geometry_and_empty_result() {
        let f = frame(Rotation::R0);
        for points in [
            [[20, 10], [80, 30], [80, 10], [20, 30]],
            [[20, 10], [20, 10], [80, 30], [20, 30]],
            [[20, 10], [201, 10], [80, 30], [20, 30]],
            [[20, 10], [80, 10], [40, 15], [20, 30]],
        ] {
            assert!(convert_page(f, &[detection(points)]).is_err());
        }
        assert_eq!(
            convert_page(f, &[]).unwrap().review_state(),
            ReviewState::NeedsInput
        );
        assert!(convert_page(Frame { width: 0, ..f }, &[]).is_err());
    }
}
