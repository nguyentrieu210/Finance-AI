use finance_knowledge::{
    guidance::{Error, ProposedCitation, extractive, validate_citations},
    repository::Hit,
};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn hit() -> Hit {
    Hit {
        chunk_id: id(1),
        policy_id: id(2),
        clause_id: id(3),
        excerpt: "Hồ sơ phải có hóa đơn và chứng từ đối chiếu.".into(),
        source_digest: "a".repeat(64),
        source_page: 4,
        distance: 0.1,
    }
}

#[test]
fn citations_are_resolved_from_pinned_evidence() {
    let hits = [hit()];
    let proposals = [ProposedCitation {
        chunk_id: id(1),
        quote: "hóa đơn",
    }];
    let result = validate_citations(id(2), &hits, &proposals).unwrap();
    assert!(result.advisory_only);
    assert_eq!(result.citations[0].clause_id, id(3));
    assert_eq!(result.citations[0].source_page, 4);
    assert_eq!(result.citations[0].source_digest, "a".repeat(64));
    assert_eq!(result.citations[0].quote, "hóa đơn");
    assert_eq!(extractive(id(2), &hits).unwrap().citations[0].quote, hits[0].excerpt);
}

#[test]
fn fabricated_quotes_and_identifiers_fail_closed() {
    let hits = [hit()];
    for (chunk_id, quote, expected) in [
        (id(9), "hóa đơn", Error::UnknownCitation),
        (id(1), "được tự động phê duyệt", Error::InvalidQuote),
        (id(1), " ", Error::InvalidQuote),
    ] {
        assert_eq!(
            validate_citations(id(2), &hits, &[ProposedCitation { chunk_id, quote }]),
            Err(expected)
        );
    }
    assert_eq!(extractive(id(9), &hits), Err(Error::WrongPolicy));
    let malformed = Hit {
        source_digest: "not-a-digest".into(),
        ..hit()
    };
    assert_eq!(extractive(id(2), &[malformed]), Err(Error::InvalidEvidence));
    let duplicates = [hit(), hit()];
    assert_eq!(extractive(id(2), &duplicates), Err(Error::InvalidEvidence));
    assert!(extractive(id(2), &[]).unwrap().citations.is_empty());
}
