use finance_extraction::gateway::{Approval, Error, Request, authorize};
use finance_shared::{
    ids::{CompanyId, CompanyScope, TenantId},
    upload_contract::Sha256Digest,
};
use uuid::Uuid;

#[test]
fn disabled_unapproved_and_tool_requests_fail_closed() {
    let scope = CompanyScope {
        tenant: TenantId::try_from(Uuid::from_u128(1)).unwrap(),
        company: CompanyId::try_from(Uuid::from_u128(2)).unwrap(),
    };
    let prompt = Sha256Digest::parse(&"a".repeat(64)).unwrap();
    let approval = Approval {
        scope,
        provider: "synthetic",
        region: "local",
        model: "fixture-v1",
        prompt,
        policy_current: true,
        synthetic_only: true,
        max_input_bytes: 4096,
        max_output_tokens: 512,
        timeout_seconds: 10,
    };
    let mut request = Request {
        scope,
        provider: "synthetic",
        region: "local",
        model: "fixture-v1",
        prompt,
        input_bytes: 100,
        output_tokens: 100,
        timeout_seconds: 5,
        synthetic: true,
        minimized: true,
        tools_enabled: false,
    };
    assert_eq!(authorize(None, &request).err(), Some(Error::Disabled));
    assert!(authorize(Some(&approval), &request).is_ok());
    request.region = "outside";
    assert_eq!(
        authorize(Some(&approval), &request).err(),
        Some(Error::UnapprovedProvider)
    );
    request.region = "local";
    request.tools_enabled = true;
    assert_eq!(
        authorize(Some(&approval), &request).err(),
        Some(Error::UnsafeRequest)
    );
    request.tools_enabled = false;
    request.synthetic = false;
    assert_eq!(
        authorize(Some(&approval), &request).err(),
        Some(Error::UnsafeRequest)
    );
    request.synthetic = true;
    request.output_tokens = 513;
    assert_eq!(
        authorize(Some(&approval), &request).err(),
        Some(Error::InvalidLimits)
    );
}
