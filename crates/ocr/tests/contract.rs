use finance_documents::{Availability, Binding, Original, evidence::Rect};
use finance_ocr::*;
use finance_shared::{
    contracts::Version,
    ids::{CompanyId, CompanyScope, ResourceId, RevisionId, TenantId},
    upload_contract::{DeclaredMime, Sha256Digest},
};

fn digest(ch: char) -> Sha256Digest {
    Sha256Digest::parse(&ch.to_string().repeat(64)).unwrap()
}

fn original(tenant: u128, company: u128, revision: u128) -> Original {
    Original {
        binding: Binding {
            scope: CompanyScope {
                tenant: TenantId::try_from(uuid::Uuid::from_u128(tenant)).unwrap(),
                company: CompanyId::try_from(uuid::Uuid::from_u128(company)).unwrap(),
            },
            document: ResourceId::try_from(uuid::Uuid::from_u128(20)).unwrap(),
            revision: RevisionId::try_from(uuid::Uuid::from_u128(revision)).unwrap(),
        },
        bytes: 100,
        digest: digest('a'),
        mime: DeclaredMime::Pdf,
        generation: Version::try_from(1).unwrap(),
        availability: Availability::Quarantined,
    }
}

fn model(version: &str, config: char) -> ModelIdentity {
    ModelIdentity::new("PaddleOCR", version, digest('b'), digest(config)).unwrap()
}

#[test]
fn cache_is_company_tenant_model_and_config_scoped() {
    let source = original(1, 10, 30);
    let key = CacheKey::new(source, model("synthetic-v1", 'c'));
    assert!(key == CacheKey::new(source, model("synthetic-v1", 'c')));
    assert!(key != CacheKey::new(original(2, 10, 30), model("synthetic-v1", 'c')));
    assert!(key != CacheKey::new(original(1, 11, 30), model("synthetic-v1", 'c')));
    assert!(key != CacheKey::new(source, model("synthetic-v2", 'c')));
    assert!(key != CacheKey::new(source, model("synthetic-v1", 'd')));
    assert!(ModelIdentity::new("", "v1", digest('b'), digest('c')).is_err());
}

#[test]
fn stale_revision_and_fence_rejected() {
    let source = original(1, 10, 30);
    let job = JobBinding::new(source, 1).unwrap();
    assert_eq!(job.check_current(job), Ok(()));
    assert_eq!(
        job.check_current(JobBinding::new(source, 2).unwrap()),
        Err(Error::StaleJob)
    );
    assert_eq!(
        job.check_current(JobBinding::new(original(1, 10, 31), 1).unwrap()),
        Err(Error::WrongSource)
    );
    let later = Original {
        generation: Version::try_from(2).unwrap(),
        ..source
    };
    assert_eq!(
        job.check_current(JobBinding::new(later, 1).unwrap()),
        Err(Error::StaleJob)
    );
    assert!(JobBinding::new(source, 0).is_err());
}

#[test]
fn vietnamese_candidate_and_quality_never_auto_accept() {
    let rect = Rect::new(0, 0, 100_000, 100_000).unwrap();
    let block = Block::new(rect, "Tổng thanh toán: 125.000 đồng", Some(10_000)).unwrap();
    assert_eq!(block.rect(), rect);
    assert_eq!(block.text(), "Tổng thanh toán: 125.000 đồng");
    assert_eq!(block.confidence(), Some(10_000));
    let page = PageResult::new(1, 2, vec![block], Some(10_000)).unwrap();
    assert_eq!(page.review_state(), ReviewState::NeedsHumanReview);
    assert_eq!(page.page(), 1);
    assert_eq!(page.blocks().len(), 1);
    assert_eq!(page.quality(), Some(10_000));
    let empty = PageResult::new(1, 1, vec![], None).unwrap();
    assert_eq!(empty.review_state(), ReviewState::NeedsInput);
    assert!(Block::new(rect, "bad\0text", None).is_err());
    assert!(Block::new(rect, "test", Some(10_001)).is_err());
    assert!(PageResult::new(0, 1, vec![], None).is_err());
    assert!(PageResult::new(1, 201, vec![], None).is_err());
    assert!(PageResult::new(1, 1, vec![], Some(10_001)).is_err());
}
