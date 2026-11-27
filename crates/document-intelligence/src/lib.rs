#![forbid(unsafe_code)]
//! E07 synthetic classification candidates. No automatic dossier routing,
//! production policy, persistence, or verification of OCR.
use finance_ocr::PageResult;
use unicode_normalization::UnicodeNormalization;

pub mod layout;
pub mod table;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Kind {
    Contract,
    ContractAppendix,
    Invoice,
    PaymentRequest,
    AdvanceRequest,
    AdvanceSettlement,
    AcceptanceRecord,
    PaymentVoucher,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Unknown,
    Ambiguous,
    Candidate(Kind),
}

/// Indexes reference the input page's original OCR block order.
/// No title text is copied into logs or a synthesized source.
#[derive(Clone, Eq, PartialEq)]
pub struct Hit {
    pub kind: Kind,
    pub page: u32,
    pub block_indexes: Vec<usize>,
}

pub struct Classification {
    pub decision: Decision,
    pub hits: Vec<Hit>,
}

impl Classification {
    pub fn requires_review(&self) -> bool {
        true
    }
}

fn normalize(text: &str) -> String {
    text.nfc()
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// Deliberately conservative, exact title candidates; no unapproved
// business semantics, fuzzy score, transliteration or accent stripping.
fn title(text: &str) -> Option<Kind> {
    match text {
        "hợp đồng" | "hợp đồng kinh tế" | "hợp đồng mua bán" => Some(Kind::Contract),
        "phụ lục hợp đồng" => Some(Kind::ContractAppendix),
        "hóa đơn" | "hóa đơn giá trị gia tăng" | "hóa đơn bán hàng" => Some(Kind::Invoice),
        "đề nghị thanh toán" | "giấy đề nghị thanh toán" => Some(Kind::PaymentRequest),
        "đề nghị tạm ứng" | "giấy đề nghị tạm ứng" => Some(Kind::AdvanceRequest),
        "quyết toán tạm ứng" | "giấy thanh toán tiền tạm ứng" => Some(Kind::AdvanceSettlement),
        "biên bản nghiệm thu" => Some(Kind::AcceptanceRecord),
        "phiếu chi" => Some(Kind::PaymentVoucher),
        _ => None,
    }
}

/// Examine only the top quarter of a source page. Multiple conflicting title
/// hits remain ambiguous. Group short OCR word blocks on the same visual row
/// using overlap with the first block; never join across columns with large gaps.
pub fn classify(page: &PageResult) -> Classification {
    let mut headers: Vec<_> = page
        .blocks()
        .iter()
        .enumerate()
        .filter(|(_, b)| b.rect().coordinates()[3] <= 250_000)
        .collect();
    headers.sort_by_key(|(index, b)| {
        let [x0, y0, _, _] = b.rect().coordinates();
        (y0, x0, *index)
    });
    let mut hits = Vec::new();
    // Standalone title blocks and contiguous row candidates are both useful.
    for (index, block) in &headers {
        if let Some(kind) = title(&normalize(block.text())) {
            hits.push(Hit {
                kind,
                page: page.page(),
                block_indexes: vec![*index],
            });
        }
    }
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for (index, block) in &headers {
        let [_, y0, _, y1] = block.rect().coordinates();
        if let Some(row) = rows.last_mut() {
            let [_, top, _, bottom] = page.blocks()[row[0]].rect().coordinates();
            let overlap = y1.min(bottom).saturating_sub(y0.max(top));
            let height = (y1 - y0).max(bottom - top);
            if overlap * 2 >= height {
                row.push(*index);
                continue;
            }
        }
        rows.push(vec![*index]);
    }
    for row in &mut rows {
        row.sort_by_key(|&i| (page.blocks()[i].rect().coordinates()[0], i));
        // Break at a large horizontal gap or overlapping box. Avoid creating
        // a phrase from two unrelated table columns or duplicate OCR layers.
        let mut group: Vec<usize> = Vec::new();
        let mut right = 0;
        for &index in row.iter() {
            let [x0, _, x1, _] = page.blocks()[index].rect().coordinates();
            if !group.is_empty() && (x0 < right || x0 - right > 35_000) {
                add_group(page, &group, &mut hits);
                group.clear();
            }
            group.push(index);
            right = x1;
        }
        add_group(page, &group, &mut hits);
    }
    // A longer exact title such as "phụ lục hợp đồng" supersedes a shorter
    // title hit only when the shorter hit is contained in the same blocks.
    let original = hits.clone();
    hits.retain(|hit| {
        !original.iter().any(|other| {
            other.block_indexes.len() > hit.block_indexes.len()
                && hit
                    .block_indexes
                    .iter()
                    .all(|i| other.block_indexes.contains(i))
        })
    });
    hits.sort_by_key(|hit| (hit.kind, hit.block_indexes.clone()));
    hits.dedup();
    let kinds: std::collections::BTreeSet<_> = hits.iter().map(|hit| hit.kind).collect();
    let decision = match kinds.len() {
        0 => Decision::Unknown,
        1 => Decision::Candidate(*kinds.first().expect("one kind")),
        _ => Decision::Ambiguous,
    };
    Classification { decision, hits }
}

fn add_group(page: &PageResult, group: &[usize], hits: &mut Vec<Hit>) {
    if group.len() < 2 || group.len() > 16 {
        return;
    }
    let text = group
        .iter()
        .map(|&i| page.blocks()[i].text())
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(kind) = title(&normalize(&text)) {
        hits.push(Hit {
            kind,
            page: page.page(),
            block_indexes: group.to_vec(),
        });
    }
}

/// A candidate boundary is not an automatic split. Every page with an exact
/// title is surfaced, even repeated kinds. Unknown pages stay unassigned;
/// never infer a contract appendix from its neighboring page.
pub fn candidate_boundaries(pages: &[PageResult]) -> Option<Vec<(u32, Decision)>> {
    if pages.is_empty() || pages.len() > 200 {
        return None;
    }
    if pages
        .iter()
        .enumerate()
        .any(|(index, page)| page.page() as usize != index + 1)
    {
        return None;
    }
    Some(
        pages
            .iter()
            .map(|page| (page.page(), classify(page).decision))
            .collect(),
    )
}
