use finance_documents::evidence::Rotation;
use finance_ocr::{
    ModelIdentity, ReviewState,
    paddle::{Detection, Frame, convert_page},
};
use finance_shared::upload_contract::Sha256Digest;
use serde::Deserialize;
use std::{fs::File, io::Read};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    points: [[u32; 2]; 4],
    text: String,
    confidence_bps: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: String,
    version: String,
    runtime: String,
    model_name: String,
    model_sha256: String,
    config_sha256: String,
    raster_sha256: String,
    width: u32,
    height: u32,
    detections: Vec<Candidate>,
}

#[test]
#[ignore = "requires real isolated PaddleOCR synthetic inference"]
fn real_paddle_vietnamese() {
    let path = std::env::var("FINANCE_PADDLE_JSON").unwrap();
    let mut bytes = Vec::new();
    File::open(path)
        .unwrap()
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .unwrap();
    assert!(bytes.len() <= 2 * 1024 * 1024);
    let result: Envelope = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(result.schema, "finance.ocr.synthetic.v1");
    assert_eq!(result.version, "3.1.1");
    assert_eq!(result.runtime, "3.0.0");
    let recognizer = std::env::var("FINANCE_PADDLE_RECOGNIZER")
        .unwrap_or_else(|_| "latin_PP-OCRv5_mobile_rec".into());
    assert_eq!(result.model_name, recognizer);
    let _model = ModelIdentity::new(
        "PaddleOCR",
        &result.version,
        Sha256Digest::parse(&result.model_sha256).unwrap(),
        Sha256Digest::parse(&result.config_sha256).unwrap(),
    )
    .unwrap();
    // Metadata provenance will require independent trusted raster verification.
    assert!(Sha256Digest::parse(&result.raster_sha256).is_ok());
    assert_eq!((result.width, result.height), (1800, 480));
    assert!(!result.detections.is_empty());
    assert!(result.detections.len() <= 10_000);
    let items: Vec<_> = result
        .detections
        .iter()
        .map(|item| Detection {
            points: item.points,
            text: &item.text,
            confidence_bps: Some(item.confidence_bps),
        })
        .collect();
    let page = convert_page(
        Frame {
            page: 1,
            page_count: 1,
            width: result.width,
            height: result.height,
            rotation: Rotation::R0,
        },
        &items,
    )
    .unwrap();
    assert_eq!(page.review_state(), ReviewState::NeedsHumanReview);
    let text = page
        .blocks()
        .iter()
        .map(|block| block.text())
        .collect::<Vec<_>>()
        .join(" ");
    // Only generated CI fixture text is permitted in this ignored probe.
    if !text.contains("125000") || !text.contains("thanh toán") {
        eprintln!(
            "synthetic OCR diagnostic: {:?}",
            text.chars().take(256).collect::<String>()
        );
    }
    assert!(text.contains("125000"), "synthetic amount not recognized");
    // Require an exact accented Vietnamese phrase plus the numeric field.
    // The official multilingual recognizer is allowed to vary on other words;
    // candidates remain human-review only and no fuzzy correction is applied.
    assert!(
        text.contains("thanh toán"),
        "synthetic accented Vietnamese phrase not recognized"
    );
    assert!(page.blocks().iter().all(|block| {
        let [x0, y0, x1, y1] = block.rect().coordinates();
        x0 < x1 && y0 < y1 && x1 <= 1_000_000 && y1 <= 1_000_000
    }));
}
