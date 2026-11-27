use finance_documents::{
    Availability, Binding, Original,
    evidence::{Cells, Error, Evidence, Locator, Page, Rect, Rotation},
};
use finance_shared::{
    contracts::Version,
    ids::{CompanyId, CompanyScope, EvidenceId, ResourceId, RevisionId, TenantId},
    upload_contract::{DeclaredMime, Sha256Digest},
};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn original(company: u128, revision: u128, digest: &str) -> Original {
    Original {
        binding: Binding {
            scope: CompanyScope {
                tenant: TenantId::try_from(id(1)).unwrap(),
                company: CompanyId::try_from(id(company)).unwrap(),
            },
            document: ResourceId::try_from(id(20)).unwrap(),
            revision: RevisionId::try_from(id(revision)).unwrap(),
        },
        bytes: 100,
        digest: Sha256Digest::parse(digest).unwrap(),
        mime: DeclaredMime::Pdf,
        generation: Version::try_from(1).unwrap(),
        availability: Availability::Quarantined,
    }
}

#[test]
fn normalized_rotation_and_bounds() {
    let r = Rect::new(100_000, 200_000, 300_000, 400_000).unwrap();
    assert_eq!(
        r.rotate(Rotation::R90).coordinates(),
        [600_000, 100_000, 800_000, 300_000]
    );
    assert_eq!(
        r.rotate(Rotation::R180).coordinates(),
        [700_000, 600_000, 900_000, 800_000]
    );
    assert_eq!(r.rotate(Rotation::R90).rotate(Rotation::R270), r);
    assert_eq!(r.rotate(Rotation::R0), r);
    assert_eq!(Rect::new(0, 0, 0, 1), Err(Error::InvalidBounds));
    assert_eq!(
        Rect::new(0, 0, 1_000_001, 1),
        Err(Error::InvalidBounds)
    );
    assert_eq!(Page::new(0, 1, r), Err(Error::InvalidPage));
    assert_eq!(Page::new(2, 1, r), Err(Error::InvalidPage));
    assert_eq!(Page::new(1, 201, r), Err(Error::InvalidPage));
}

#[test]
fn spreadsheet_bounds_and_sheet_name() {
    let cells = Cells::new("Chi phí", 1, 1, 1_048_576, 16_384).unwrap();
    assert_eq!(cells.sheet(), "Chi phí");
    assert_eq!(cells.range(), [1, 1, 1_048_576, 16_384]);
    assert!(Cells::new("bad/sheet", 1, 1, 1, 1).is_err());
    assert!(Cells::new("sheet", 0, 1, 1, 1).is_err());
    assert!(Cells::new("sheet", 2, 1, 1, 1).is_err());
    assert!(Cells::new("sheet", 1, 1, 1_048_577, 1).is_err());
    assert!(Cells::new("sheet", 1, 1, 1, 16_385).is_err());
}

#[test]
fn evidence_is_bound_to_original_and_revision() {
    let digest = "a".repeat(64);
    let source = original(10, 30, &digest);
    let page = Page::new(1, 2, Rect::new(0, 0, 10, 10).unwrap()).unwrap();
    let evidence_id = EvidenceId::try_from(id(50)).unwrap();
    let evidence = Evidence::new(
        evidence_id,
        source,
        Locator::Page(page),
        "Tổng tiền: 123",
    )
    .unwrap();
    assert_eq!(evidence.id(), evidence_id);
    assert_eq!(evidence.excerpt(), "Tổng tiền: 123");
    assert_eq!(evidence.check_source(source), Ok(()));
    assert_eq!(
        evidence.check_source(original(11, 30, &digest)),
        Err(Error::WrongSource)
    );
    assert_eq!(
        evidence.check_source(original(10, 31, &digest)),
        Err(Error::WrongSource)
    );
    assert_eq!(
        evidence.check_source(original(10, 30, &"b".repeat(64))),
        Err(Error::WrongSource)
    );
    assert!(matches!(
        Evidence::new(evidence_id, source, Locator::Page(page), "bad\0text"),
        Err(Error::InvalidText)
    ));
    assert!(matches!(
        Evidence::new(
            evidence_id,
            source,
            Locator::Cells(Cells::new("sheet", 1, 1, 1, 1).unwrap()),
            "123"
        ),
        Err(Error::WrongFormat)
    ));
}
