use finance_ocr::{ReviewState, tsv::parse_page};
use std::fs;

#[test]
#[ignore = "requires synthetic Tesseract fixture from GitHub CI"]
fn real_vietnamese_tsv() {
    let path = std::env::var("FINANCE_OCR_TSV").unwrap();
    let input = fs::read_to_string(path).unwrap();
    let page = parse_page(&input, 1, 1, 1800, 480).unwrap();
    assert_eq!(page.review_state(), ReviewState::NeedsHumanReview);
    let text = page
        .blocks()
        .iter()
        .map(|block| block.text())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains("125000"));
    assert!(text.contains("Tổng"));
    assert!(text.contains("thanh toán"));
    assert!(page.blocks().iter().all(|block| {
        let [x0, y0, x1, y1] = block.rect().coordinates();
        x0 < x1 && y0 < y1 && x1 <= 1_000_000 && y1 <= 1_000_000
    }));
}
