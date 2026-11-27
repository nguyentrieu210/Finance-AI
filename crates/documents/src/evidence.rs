//! Internal evidence-coordinate prototype; no parser, persistence or viewer.
use crate::{Binding, Original};
use finance_shared::{
    ids::EvidenceId,
    upload_contract::{DeclaredMime, Sha256Digest},
};

const SCALE: u32 = 1_000_000;
const MAX_ROWS: u32 = 1_048_576;
const MAX_COLUMNS: u32 = 16_384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidBounds,
    InvalidPage,
    InvalidCell,
    InvalidText,
    WrongSource,
    WrongFormat,
}

/// Top-left normalized rectangle, integer millionths of the unrotated page.
/// Exact arithmetic; a nonempty rectangle must stay within [0, 1].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rotation {
    R0,
    R90,
    R180,
    R270,
}

impl Rect {
    pub fn new(x0: u32, y0: u32, x1: u32, y1: u32) -> Result<Self, Error> {
        if x0 >= x1 || y0 >= y1 || x1 > SCALE || y1 > SCALE {
            return Err(Error::InvalidBounds);
        }
        Ok(Self { x0, y0, x1, y1 })
    }

    pub fn coordinates(self) -> [u32; 4] {
        [self.x0, self.y0, self.x1, self.y1]
    }

    /// Clockwise transform to display orientation; not a second PDF rotation.
    pub fn rotate(self, rotation: Rotation) -> Self {
        let Self { x0, y0, x1, y1 } = self;
        let (x0, y0, x1, y1) = match rotation {
            Rotation::R0 => (x0, y0, x1, y1),
            Rotation::R90 => (SCALE - y1, x0, SCALE - y0, x1),
            Rotation::R180 => (SCALE - x1, SCALE - y1, SCALE - x0, SCALE - y0),
            Rotation::R270 => (y0, SCALE - x1, y1, SCALE - x0),
        };
        Self { x0, y0, x1, y1 }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Page {
    index: u32,
    rect: Rect,
}

impl Page {
    /// page_count must come from the trusted bounded parser, not client input.
    pub fn new(index: u32, page_count: u32, rect: Rect) -> Result<Self, Error> {
        if index == 0 || index > page_count || page_count > 200 {
            return Err(Error::InvalidPage);
        }
        Ok(Self { index, rect })
    }

    pub fn index(self) -> u32 {
        self.index
    }

    pub fn rect(self) -> Rect {
        self.rect
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct Cells {
    sheet: String,
    first_row: u32,
    first_column: u32,
    last_row: u32,
    last_column: u32,
}

impl Cells {
    /// 1-based inclusive cell range; no formula, external link or executable
    /// worksheet reference. Existence of the named sheet requires the parser.
    pub fn new(
        sheet: &str,
        first_row: u32,
        first_column: u32,
        last_row: u32,
        last_column: u32,
    ) -> Result<Self, Error> {
        if sheet.is_empty()
            || sheet.encode_utf16().count() > 31
            || sheet.chars().any(char::is_control)
            || sheet.contains([':', '/', '\\', '?', '*', '[', ']'])
            || first_row == 0
            || first_column == 0
            || first_row > last_row
            || first_column > last_column
            || last_row > MAX_ROWS
            || last_column > MAX_COLUMNS
        {
            return Err(Error::InvalidCell);
        }
        Ok(Self {
            sheet: sheet.to_owned(),
            first_row,
            first_column,
            last_row,
            last_column,
        })
    }

    pub fn sheet(&self) -> &str {
        &self.sheet
    }

    pub fn range(&self) -> [u32; 4] {
        [
            self.first_row,
            self.first_column,
            self.last_row,
            self.last_column,
        ]
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum Locator {
    Page(Page),
    Cells(Cells),
}

/// Candidate only. No Deserialize or Debug (excerpt must not enter logs).
/// This type does not establish that a parser found the quoted text.
pub struct Evidence {
    id: EvidenceId,
    binding: Binding,
    source_digest: Sha256Digest,
    locator: Locator,
    excerpt: String,
}

impl Evidence {
    pub fn new(
        id: EvidenceId,
        original: Original,
        locator: Locator,
        excerpt: &str,
    ) -> Result<Self, Error> {
        let compatible = match &locator {
            Locator::Page(page) => match original.mime {
                DeclaredMime::Pdf => true,
                DeclaredMime::Jpeg | DeclaredMime::Png => page.index() == 1,
                DeclaredMime::Docx | DeclaredMime::Xlsx => false,
            },
            Locator::Cells(_) => original.mime == DeclaredMime::Xlsx,
        };
        if !compatible {
            return Err(Error::WrongFormat);
        }
        if excerpt.trim().is_empty()
            || excerpt.len() > 4096
            || excerpt
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err(Error::InvalidText);
        }
        Ok(Self {
            id,
            binding: original.binding,
            source_digest: original.digest,
            locator,
            excerpt: excerpt.to_owned(),
        })
    }

    pub fn id(&self) -> EvidenceId {
        self.id
    }

    pub fn check_source(&self, original: Original) -> Result<(), Error> {
        if self.binding != original.binding || self.source_digest != original.digest {
            return Err(Error::WrongSource);
        }
        Ok(())
    }

    pub fn locator(&self) -> &Locator {
        &self.locator
    }

    pub fn excerpt(&self) -> &str {
        &self.excerpt
    }
}
