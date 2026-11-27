use finance_document_intelligence::{Decision, Kind, candidate_boundaries, classify};
use finance_documents::evidence::Rect;
use finance_ocr::{Block, PageResult};

fn block(text: &str, x: u32, y: u32, width: u32) -> Block {
    Block::new(
        Rect::new(x, y, x + width, y + 20_000).unwrap(),
        text,
        Some(9500),
    )
    .unwrap()
}

#[test]
fn exact_titles_preserve_raw_text_and_block_provenance() {
    let raw = "  HO\u{31b}\u{323}P ĐÔ\u{300}NG  ";
    let page = PageResult::new(
        1,
        1,
        vec![
            block(raw, 20_000, 60_000, 200_000),
            block("HÓA ĐƠN", 20_000, 600_000, 200_000),
        ],
        None,
    )
    .unwrap();
    let result = classify(&page);
    assert_eq!(result.decision, Decision::Candidate(Kind::Contract));
    assert!(result.requires_review());
    assert_eq!(result.hits[0].block_indexes, vec![0]);
    assert_eq!(result.hits[0].page, 1);
    assert_eq!(page.blocks()[0].text(), raw);
}

#[test]
fn split_words_and_conflicting_titles() {
    let blocks = vec![
        block("PHỤ LỤC", 100_000, 80_000, 100_000),
        block("HỢP ĐỒNG", 210_000, 80_000, 100_000),
    ];
    let page = PageResult::new(1, 1, blocks, None).unwrap();
    let result = classify(&page);
    assert_eq!(result.decision, Decision::Candidate(Kind::ContractAppendix));
    assert_eq!(result.hits[0].block_indexes, vec![0, 1]);
    let page = PageResult::new(
        1,
        1,
        vec![
            block("HỢP ĐỒNG", 20_000, 30_000, 100_000),
            block("HÓA ĐƠN", 20_000, 100_000, 100_000),
        ],
        None,
    )
    .unwrap();
    assert_eq!(classify(&page).decision, Decision::Ambiguous);
}

#[test]
fn unknown_pages_and_multi_document_boundaries() {
    let pages = vec![
        PageResult::new(1, 3, vec![block("HỢP ĐỒNG", 10_000, 20_000, 100_000)], None).unwrap(),
        PageResult::new(2, 3, vec![], None).unwrap(),
        PageResult::new(
            3,
            3,
            vec![block("PHỤ LỤC HỢP ĐỒNG", 10_000, 20_000, 300_000)],
            None,
        )
        .unwrap(),
    ];
    assert_eq!(
        candidate_boundaries(&pages),
        Some(vec![
            (1, Decision::Candidate(Kind::Contract)),
            (2, Decision::Unknown),
            (3, Decision::Candidate(Kind::ContractAppendix)),
        ])
    );
    assert!(candidate_boundaries(&pages[1..]).is_none());
    let unrelated = PageResult::new(
        1,
        1,
        vec![block("Theo hợp đồng số 12", 10_000, 20_000, 300_000)],
        None,
    )
    .unwrap();
    assert_eq!(classify(&unrelated).decision, Decision::Unknown);
}
