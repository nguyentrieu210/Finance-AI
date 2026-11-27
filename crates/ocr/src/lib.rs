#![forbid(unsafe_code)]
//! Synthetic OCR result-domain prototype. No parser, scan release, provider
//! execution, storage access, authoritative extraction or public API.
use finance_documents::{Binding, Original, evidence::Rect};
use finance_shared::{ids::CompanyScope, upload_contract::Sha256Digest};

pub mod tsv;

const MAX_PAGES: u32 = 200;
const MAX_BLOCKS: usize = 10_000;
const MAX_PAGE_TEXT_BYTES: usize = 262_144;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidModel,
    InvalidPage,
    InvalidBlock,
    TooMuchText,
    WrongSource,
    StaleJob,
    InvalidQuality,
}

/// Immutable artifact identity includes the model weights and all preprocessing
/// and recognition settings. Names alone do not identify model behavior.
#[derive(Clone, Eq, PartialEq)]
pub struct ModelIdentity {
    engine: String,
    version: String,
    weights: Sha256Digest,
    config: Sha256Digest,
}

impl ModelIdentity {
    pub fn new(
        engine: &str,
        version: &str,
        weights: Sha256Digest,
        config: Sha256Digest,
    ) -> Result<Self, Error> {
        if ![engine, version].iter().all(|s| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        }) {
            return Err(Error::InvalidModel);
        }
        Ok(Self {
            engine: engine.to_owned(),
            version: version.to_owned(),
            weights,
            config,
        })
    }
}

/// Scope deliberately includes company as well as tenant. Cache cannot be
/// used to authorize access; attach cached data only after fresh authorization.
#[derive(Clone, Eq, PartialEq)]
pub struct CacheKey {
    scope: CompanyScope,
    source: Sha256Digest,
    model: ModelIdentity,
}

impl CacheKey {
    pub fn new(original: Original, model: ModelIdentity) -> Self {
        Self {
            scope: original.binding.scope,
            source: original.digest,
            model,
        }
    }
}

/// Fenced source for a synthetic worker result. Trusted job repository must
/// compare these values transactionally before any future persistence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobBinding {
    source: Binding,
    digest: Sha256Digest,
    generation: i64,
    fence: u64,
}

impl JobBinding {
    pub fn new(original: Original, fence: u64) -> Result<Self, Error> {
        let generation = i64::from(original.generation);
        if fence == 0 || generation == 0 {
            return Err(Error::StaleJob);
        }
        Ok(Self {
            source: original.binding,
            digest: original.digest,
            generation,
            fence,
        })
    }

    pub fn check_current(self, current: Self) -> Result<(), Error> {
        if self.source != current.source || self.digest != current.digest {
            return Err(Error::WrongSource);
        }
        if self.generation != current.generation || self.fence != current.fence {
            return Err(Error::StaleJob);
        }
        Ok(())
    }
}

/// Coordinates are normalized using E05's exact rectangle representation.
/// Confidence is optional and expressed in basis points, never proof of fact.
pub struct Block {
    rect: Rect,
    text: String,
    confidence: Option<u16>,
}

impl Block {
    pub fn new(rect: Rect, text: &str, confidence: Option<u16>) -> Result<Self, Error> {
        if text.trim().is_empty()
            || text.len() > 4096
            || text
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
            || confidence.is_some_and(|c| c > 10_000)
        {
            return Err(Error::InvalidBlock);
        }
        Ok(Self {
            rect,
            text: text.to_owned(),
            confidence,
        })
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn confidence(&self) -> Option<u16> {
        self.confidence
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewState {
    NeedsInput,
    NeedsHumanReview,
}

/// Optional synthetic image quality signal. No policy threshold or automatic
/// acceptance: every nonempty candidate still requires human review.
pub struct PageResult {
    page: u32,
    blocks: Vec<Block>,
    quality_bps: Option<u16>,
}

impl PageResult {
    pub fn new(
        page: u32,
        page_count: u32,
        blocks: Vec<Block>,
        quality_bps: Option<u16>,
    ) -> Result<Self, Error> {
        if page == 0 || page > page_count || page_count > MAX_PAGES {
            return Err(Error::InvalidPage);
        }
        if blocks.len() > MAX_BLOCKS
            || blocks.iter().map(|b| b.text.len()).sum::<usize>() > MAX_PAGE_TEXT_BYTES
        {
            return Err(Error::TooMuchText);
        }
        if quality_bps.is_some_and(|v| v > 10_000) {
            return Err(Error::InvalidQuality);
        }
        Ok(Self {
            page,
            blocks,
            quality_bps,
        })
    }

    pub fn page(&self) -> u32 {
        self.page
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn quality(&self) -> Option<u16> {
        self.quality_bps
    }

    pub fn review_state(&self) -> ReviewState {
        if self.blocks.is_empty() {
            ReviewState::NeedsInput
        } else {
            ReviewState::NeedsHumanReview
        }
    }
}
