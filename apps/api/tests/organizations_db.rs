use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Path, State},
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use finance_api::organizations::{OrganizationState, router};
use finance_identity::{Verifier, VerifierStore};
use finance_shared::ids::{CompanyId, CompanyScope, TenantId};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use tower::ServiceExt;
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

async fn request(app: &Router, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut builder = Request::builder().uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap();
    (status, value)
}

async fn resolve_risk(
    app: &Router,
    token: &str,
    risk: &str,
    key: &str,
    version: &str,
) -> (StatusCode, Value, Option<String>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/risks/{risk}/resolve"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", key)
                .header("if-finance-version", version)
                .body(Body::from(
                    json!({
                        "reason":"Verified assigned-accountant risk closure",
                        "evidence_ids":["00000000-0000-0000-0000-000000002333"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap();
    (status, value, receipt)
}

async fn create_organization(
    app: &Router,
    token: &str,
    key: &str,
    body: Value,
) -> (StatusCode, Value, Option<String>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/organizations")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", key)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let version = response
        .headers()
        .get("finance-version")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value = serde_json::from_slice(&bytes).unwrap();
    (status, value, version)
}

// Test-only static file serving; never mounted in the production binary.
async fn index(State(root): State<PathBuf>) -> Response {
    match std::fs::read(root.join("index.html")) {
        Ok(data) => ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], data).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn asset(State(root): State<PathBuf>, Path(name): Path<String>) -> Response {
    if name.len() > 128
        || name.starts_with('.')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mime = if name.ends_with(".js") {
        "text/javascript"
    } else if name.ends_with(".css") {
        "text/css"
    } else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match std::fs::read(root.join("assets").join(name)) {
        Ok(data) => ([(header::CONTENT_TYPE, mime)], data).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires disposable PostgreSQL and synthetic test credentials"]
async fn scoped_organization_http_real_database() {
    let runtime = PgPoolOptions::new()
        .max_connections(3)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE finance_runtime")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&std::env::var("FINANCE_PROTOTYPE_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let lookup = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE finance_identity_lookup")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&std::env::var("FINANCE_IDENTITY_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let key = RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).unwrap();
    let jwks = json!({"keys":[{"kty":"RSA","kid":"http-test","alg":"RS256","n":URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()),"e":URL_SAFE_NO_PAD.encode(key.e().to_bytes_be())}]}).to_string();
    let verifier = VerifierStore::new(
        Verifier::from_trusted_snapshot(
            "https://synthetic.invalid",
            "finance-api",
            &jwks,
            now(),
            600,
        )
        .unwrap(),
    );
    let signer = EncodingKey::from_rsa_der(key.to_pkcs1_der().unwrap().as_bytes());
    let mint = |sub: &str| {
        let mut header = Header::new(Algorithm::RS256);
        header.typ = Some("at+jwt".into());
        header.kid = Some("http-test".into());
        let time = now();
        encode(&header, &json!({"iss":"https://synthetic.invalid","sub":sub,"aud":"finance-api","iat":time,"exp":time+300,"role":"ORG_ADMIN"}), &signer).unwrap()
    };
    let token = mint("org-admin");
    let auditor = mint("auditor");
    let requester = mint("requester");
    let other_requester = mint("second-requester");
    let policy_admin = mint("policy-admin");
    let review_accountant = mint("review-accountant");
    let scope = CompanyScope {
        tenant: TenantId::try_from(id(1)).unwrap(),
        company: CompanyId::try_from(id(10)).unwrap(),
    };
    let app = router(OrganizationState::synthetic(
        runtime.clone(),
        lookup.clone(),
        verifier,
        scope,
        [71; 32],
    ));
    let membership_subject = "00000000-0000-0000-0000-0000000003ef";
    let (membership_status, membership, membership_version) = create_membership(
        &app,
        &token,
        "membership-create-one",
        membership_subject,
        &["REQUESTER", "AUDITOR"],
    )
    .await;
    assert_eq!(membership_status, StatusCode::CREATED);
    assert_eq!(membership["id"], membership_subject);
    assert_eq!(membership["subject_id"], membership_subject);
    assert_eq!(membership["active"], true);
    assert_eq!(membership["roles"].as_array().unwrap().len(), 2);
    let membership_uri = format!("/v1/memberships/{membership_subject}");
    assert_eq!(
        request(&app, &membership_uri, Some(&token)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, "/v1/memberships", Some(&requester)).await.0,
        StatusCode::FORBIDDEN
    );
    let (revoke_status, revoked, revoked_version) = revoke_membership(
        &app,
        &token,
        membership_subject,
        "membership-revoke-one",
        membership_version.as_deref().unwrap(),
    )
    .await;
    assert_eq!(revoke_status, StatusCode::OK);
    assert_eq!(revoked["active"], false);
    assert_ne!(membership_version, revoked_version);
    assert_eq!(
        revoke_membership(
            &app,
            &token,
            "00000000-0000-0000-0000-0000000003e8",
            "membership-self-revoke",
            "\"v1:membership:00000000-0000-0000-0000-0000000003e8:0\"",
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &app,
            "/v1/organizations/00000000-0000-0000-0000-000000000065",
            Some(&token)
        )
        .await
        .0,
        StatusCode::OK
    );
    let org_body = json!({
        "kind": "DEPARTMENT",
        "code": "HTTP-ORG-CREATE",
        "name": "HTTP organization create"
    });
    let (org_status, created_org, org_version) =
        create_organization(&app, &token, "http-org-create", org_body.clone()).await;
    assert_eq!(org_status, StatusCode::CREATED);
    assert_eq!(created_org["code"], "HTTP-ORG-CREATE");
    assert_eq!(created_org["active"], true);
    assert!(
        org_version
            .as_deref()
            .is_some_and(|value| value.ends_with(":0\""))
    );
    let replay = create_organization(&app, &token, "http-org-create", org_body.clone()).await;
    assert_eq!(replay.0, StatusCode::CREATED);
    assert_eq!(replay.1["id"], created_org["id"]);
    assert_eq!(replay.2, org_version);
    assert_eq!(
        create_organization(
            &app,
            &token,
            "http-org-create",
            json!({"kind":"DEPARTMENT","code":"HTTP-ORG-CHANGED","name":"changed"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        create_organization(
            &app,
            &requester,
            "http-org-requester",
            json!({"kind":"DEPARTMENT","code":"DENIED","name":"denied"})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        create_organization(
            &app,
            &token,
            "http-company-create",
            json!({"kind":"COMPANY","code":"NOPE","name":"tenant-level denied"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(
            &app,
            "/v1/organizations/00000000-0000-0000-0000-000000000401",
            Some(&token)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    for (uri, kind) in [
        ("/v1/contracts?limit=10", "CONTRACT"),
        ("/v1/advances?limit=10", "ADVANCE"),
        ("/v1/budgets?limit=10", "BUDGET"),
    ] {
        let (status, page) = request(&app, uri, Some(&requester)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert_eq!(page["items"][0]["kind"], kind);
        assert_eq!(page["items"][0]["coverage"], "COMPLETE");
        assert_eq!(page["items"][0]["state"], "VERIFIED");
        assert_eq!(
            request(&app, uri, Some(&auditor)).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(request(&app, uri, None).await.0, StatusCode::UNAUTHORIZED);
    }
    let (reference_status, reference_page) =
        request(&app, "/v1/references?limit=100", Some(&requester)).await;
    assert_eq!(reference_status, StatusCode::OK);
    let reference_items = reference_page["items"].as_array().unwrap();
    for kind in ["CONTRACT", "ADVANCE", "BUDGET"] {
        assert!(reference_items.iter().any(|item| item["kind"] == kind));
    }
    assert!(
        reference_items
            .iter()
            .all(|item| { item["state"] == "VERIFIED" && item["coverage"] == "COMPLETE" })
    );
    for item in reference_items {
        let uri = format!("/v1/references/{}", item["id"].as_str().unwrap());
        let (detail_status, reference_detail) = request(&app, &uri, Some(&requester)).await;
        assert_eq!(detail_status, StatusCode::OK);
        assert_eq!(&reference_detail, item);
        assert_eq!(
            request(&app, &uri, Some(&auditor)).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(request(&app, &uri, None).await.0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(
        request(
            &app,
            "/v1/references/ffffffff-ffff-ffff-ffff-ffffffffffff",
            Some(&requester),
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, first_reference) = request(&app, "/v1/references?limit=1", Some(&requester)).await;
    let reference_cursor = first_reference["next_cursor"].as_str().unwrap();
    let (next_reference_status, next_reference) = request(
        &app,
        &format!("/v1/references?limit=1&cursor={reference_cursor}"),
        Some(&requester),
    )
    .await;
    assert_eq!(next_reference_status, StatusCode::OK);
    assert_ne!(
        first_reference["items"][0]["id"],
        next_reference["items"][0]["id"]
    );
    assert_eq!(
        request(
            &app,
            &format!("/v1/contracts?limit=1&cursor={reference_cursor}"),
            Some(&requester),
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(&app, "/v1/references", Some(&auditor)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&app, "/v1/references", None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let download_uri = "/v1/dossiers/00000000-0000-0000-0000-000000002320/documents/00000000-0000-0000-0000-000000002330/download";
    let (download_status, download) = request(&app, download_uri, Some(&requester)).await;
    assert_eq!(download_status, StatusCode::OK);
    let content_uri = download["url"].as_str().unwrap();
    let content_request = |actor: &str, uri: &str| {
        Request::builder()
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {actor}"))
            .body(Body::empty())
            .unwrap()
    };
    let denied = app
        .clone()
        .oneshot(content_request(&review_accountant, content_uri))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let bad_uri = format!("{content_uri}x");
    let denied = app
        .clone()
        .oneshot(content_request(&requester, &bad_uri))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let result = app
        .clone()
        .oneshot(content_request(&requester, content_uri))
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    assert_eq!(result.headers()[header::CACHE_CONTROL], "no-store");
    let bytes = to_bytes(result.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(bytes.as_ref(), b"%PDF-1.4\nsynthetic risk evidence");
    assert_eq!(
        request(&app, download_uri, Some(&other_requester)).await.0,
        StatusCode::NOT_FOUND
    );
    let evidence_uri = "/v1/evidence/00000000-0000-0000-0000-000000002333";
    for reader in [&requester, &review_accountant] {
        let (evidence_status, evidence) = request(&app, evidence_uri, Some(reader)).await;
        assert_eq!(evidence_status, StatusCode::OK);
        assert_eq!(evidence["page"], 1);
        assert_eq!(evidence["sha256"].as_str().unwrap().len(), 64);
        assert_eq!(
            evidence["excerpt"],
            "Synthetic bank account change evidence"
        );
        assert_eq!(evidence["locator"], "rect:1000,1000,500000,50000");
    }
    assert_eq!(
        request(&app, evidence_uri, Some(&other_requester)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, evidence_uri, Some(&auditor)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, evidence_uri, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (audit_status, audit_page) = request(&app, "/v1/audit?limit=2", Some(&auditor)).await;
    assert_eq!(audit_status, StatusCode::OK);
    assert!(audit_page["items"].is_array());
    assert_eq!(
        request(&app, "/v1/audit", Some(&requester)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&app, "/v1/audit", None).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (policy_status, policy_page) = request(&app, "/v1/policies", Some(&requester)).await;
    assert_eq!(policy_status, StatusCode::OK);
    let policies = policy_page["items"].as_array().unwrap();
    assert!(!policies.is_empty());
    assert!(policies.iter().all(|policy| policy["state"] == "PUBLISHED"));
    let policy_id = policies
        .iter()
        .find(|policy| policy["id"] == "00000000-0000-0000-0000-000000001600")
        .and_then(|policy| policy["id"].as_str())
        .expect("published settlement policy fixture must be visible");
    let (get_status, policy) =
        request(&app, &format!("/v1/policies/{policy_id}"), Some(&requester)).await;
    assert_eq!(get_status, StatusCode::OK);
    assert_eq!(policy["state"], "PUBLISHED");
    let invalid_policy = Request::builder()
        .method("POST")
        .uri("/v1/policies")
        .header(header::AUTHORIZATION, format!("Bearer {policy_admin}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", "policy-missing-source")
        .body(Body::from(
            json!({
                "company_id":"00000000-0000-0000-0000-00000000000a",
                "name":"Synthetic source test",
                "dossier_kind":"PAYMENT",
                "effective_from":"2033-01-01T00:00:00Z",
                "effective_to":"2034-01-01T00:00:00Z",
                "source_document_id":"ffffffff-ffff-ffff-ffff-ffffffffffff",
                "clauses":[{"key":"source","evidence_id":"ffffffff-ffff-ffff-ffff-ffffffffffff"}],
                "checklist":[{"clause_key":"source","document_kind":"INVOICE","mandatory":true}]
            })
            .to_string(),
        ))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(invalid_policy).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let policy_action = |op: &str, actor: &str, key: &str| {
        Request::builder()
            .method("POST")
            .uri(format!("/v1/policies/{policy_id}/{op}"))
            .header(header::AUTHORIZATION, format!("Bearer {actor}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header("idempotency-key", key)
            .header(
                "if-finance-version",
                format!("\"v1:policy:{policy_id}:{}\"", policy["version"]),
            )
            .body(Body::from(
                json!({"reason":"Synthetic policy test"}).to_string(),
            ))
            .unwrap()
    };
    let denied = app
        .clone()
        .oneshot(policy_action("revoke", &requester, "policy-denied"))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let denied = app
        .clone()
        .oneshot(policy_action("submit", &requester, "policy-submit-denied"))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        request(&app, "/v1/policies", Some(&auditor)).await.0,
        StatusCode::FORBIDDEN
    );
    let (knowledge_status, hits) = request(
        &app,
        "/__diagnostics/knowledge?kind=SETTLEMENT&day=2031-06-01&q=settlement&limit=10",
        Some(&requester),
    )
    .await;
    assert_eq!(knowledge_status, StatusCode::OK);
    assert_eq!(hits.as_array().unwrap().len(), 1);
    assert_eq!(hits[0]["source_page"], 1);
    assert!(hits[0]["excerpt"].as_str().unwrap().contains("settlement"));
    assert_eq!(
        request(
            &app,
            "/__diagnostics/knowledge?kind=SETTLEMENT&day=2030-06-01&q=settlement&limit=10",
            Some(&requester),
        )
        .await
        .1
        .as_array()
        .unwrap()
        .len(),
        0
    );
    assert_eq!(
        request(
            &app,
            "/__diagnostics/knowledge?kind=SETTLEMENT&day=2031-06-01&q=settlement",
            Some(&auditor),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let transitions = json!([
        {"command":"PROCESS","from":"DRAFT","to":"PROCESSING","role":"REQUESTER","guard_refs":["authorized","current_revision","clean_docs"]},
        {"command":"PROCESS","from":"NEEDS_INPUT","to":"PROCESSING","role":"REQUESTER","guard_refs":["authorized","current_revision","clean_docs"]},
        {"command":"PROCESS_COMPLETE","from":"PROCESSING","to":"REVIEWABLE","role":"SYSTEM","guard_refs":["authorized","current_revision","worker_fenced","complete"]},
        {"command":"PROCESS_COMPLETE","from":"PROCESSING","to":"NEEDS_INPUT","role":"SYSTEM","guard_refs":["authorized","current_revision","worker_fenced","incomplete"]},
        {"command":"READY","from":"REVIEWABLE","to":"READY","role":"REQUESTER","guard_refs":["authorized","current_revision","complete","verified","rules_pass","reserved","snapshot_current"]},
        {"command":"EXPORT","from":"READY","to":"EXPORTED","role":"SYSTEM","guard_refs":["authorized","current_revision","artifact_verified","snapshot_current"]},
        {"command":"HANDOFF","from":"EXPORTED","to":"HANDED_OFF","role":"REQUESTER","guard_refs":["authorized","current_revision","external_receipt","reserved","snapshot_current","rules_pass"]},
        {"command":"BEGIN_REVIEW","from":"HANDED_OFF","to":"ACCOUNTING_REVIEW","role":"REQUESTER","guard_refs":["authorized","current_revision","independent_accountant"]},
        {"command":"ACCEPT","from":"ACCOUNTING_REVIEW","to":"ACCEPTED","role":"ACCOUNTANT","guard_refs":["authorized","current_revision","independent_accountant","critical_confirmed","required_external_evidence","snapshot_current","rules_pass"]},
        {"command":"RETURN","from":"ACCOUNTING_REVIEW","to":"RETURNED","role":"ACCOUNTANT","guard_refs":["authorized","current_revision","independent_accountant","return_reason","reconciled"]},
        {"command":"REOPEN","from":"RETURNED","to":"DRAFT","role":"REQUESTER","guard_refs":["authorized","current_revision","reconciled"]}
    ]);
    let (workflow_status, workflow, workflow_token) = create_workflow(
        &app,
        &policy_admin,
        "workflow-one",
        "Synthetic workflow",
        transitions.clone(),
    )
    .await;
    assert_eq!(workflow_status, StatusCode::CREATED);
    assert_eq!(workflow["version"], 1);
    let workflow_id = workflow["id"].as_str().unwrap();
    let workflow_uri = format!("/v1/workflows/{workflow_id}");
    assert_eq!(
        request(&app, &workflow_uri, Some(&requester)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, &workflow_uri, Some(&policy_admin)).await.1,
        workflow
    );
    assert_eq!(
        create_workflow(
            &app,
            &requester,
            "workflow-denied",
            "Denied",
            transitions.clone(),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (revise_workflow_status, revised_workflow, revised_workflow_token) = revise_workflow(
        &app,
        &policy_admin,
        workflow_id,
        "workflow-revise",
        workflow_token.as_deref().unwrap(),
        ("Synthetic workflow v2", transitions.clone(), "Revision"),
    )
    .await;
    assert_eq!(revise_workflow_status, StatusCode::OK);
    assert_eq!(revised_workflow["version"], 2);
    assert_eq!(
        create_workflow(
            &app,
            &policy_admin,
            "workflow-one",
            "Synthetic workflow",
            transitions.clone(),
        )
        .await
        .1,
        workflow
    );
    assert_eq!(
        revise_workflow(
            &app,
            &policy_admin,
            workflow_id,
            "workflow-stale",
            workflow_token.as_deref().unwrap(),
            ("Stale", transitions.clone(), "Stale revision"),
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (third_status, third_workflow, _) = revise_workflow(
        &app,
        &policy_admin,
        workflow_id,
        "workflow-revise-three",
        revised_workflow_token.as_deref().unwrap(),
        (
            "Synthetic workflow v3",
            transitions.clone(),
            "Revision three",
        ),
    )
    .await;
    assert_eq!(third_status, StatusCode::OK);
    assert_eq!(third_workflow["version"], 3);
    let (replay_status, replay_body, replay_token) = revise_workflow(
        &app,
        &policy_admin,
        workflow_id,
        "workflow-revise",
        workflow_token.as_deref().unwrap(),
        ("Synthetic workflow v2", transitions.clone(), "Revision"),
    )
    .await;
    assert_eq!(replay_status, StatusCode::OK);
    assert_eq!(replay_body, revised_workflow);
    assert_eq!(replay_token, revised_workflow_token);
    let (workflow_list_status, workflow_page) =
        request(&app, "/v1/workflows", Some(&policy_admin)).await;
    assert_eq!(workflow_list_status, StatusCode::OK);
    assert!(
        workflow_page["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["id"] == workflow_id && item["version"] == 3 })
    );
    let fields = json!([
        {"path":"advance","label":"Có tạm ứng","type":"boolean","required":true,"source_path":"request.advance","layout_row":0,"layout_column":0,"layout_width":12},
        {"path":"amount","label":"Số tiền","type":"money","required":true,"source_path":"request.amount","layout_row":1,"layout_column":0,"layout_width":6,"condition_ref":"advance"},
        {"path":"date","label":"Ngày đề nghị","type":"date","required":true}
    ]);
    let (form_status, form, form_token) = create_form(
        &app,
        &policy_admin,
        "form-one",
        "Synthetic payment form",
        fields.clone(),
    )
    .await;
    assert_eq!(form_status, StatusCode::CREATED);
    assert_eq!(form["version"], 1);
    assert_eq!(form["state"], "DRAFT");
    let form_id = form["id"].as_str().unwrap().to_owned();
    assert_eq!(
        create_form(
            &app,
            &policy_admin,
            "form-one",
            "Synthetic payment form",
            fields.clone()
        )
        .await
        .1,
        form
    );
    assert_eq!(
        create_form(
            &app,
            &policy_admin,
            "form-one",
            "Changed name",
            fields.clone()
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        create_form(&app, &requester, "form-denied", "Denied", fields.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (revise_status, revised, revise_token) = revise_form(
        &app,
        &policy_admin,
        &form_id,
        "form-revise",
        form_token.as_deref().unwrap(),
        (
            "Synthetic payment form v2",
            fields.clone(),
            "Add stable source mapping",
        ),
    )
    .await;
    assert_eq!(revise_status, StatusCode::OK);
    assert_eq!(revised["id"], form["id"]);
    assert_eq!(revised["version"], 2);
    assert_ne!(form_token, revise_token);
    assert_eq!(
        revise_form(
            &app,
            &policy_admin,
            &form_id,
            "form-stale",
            form_token.as_deref().unwrap(),
            ("stale", fields.clone(), "stale attempt")
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (_, form_page) = request(&app, "/v1/forms", Some(&policy_admin)).await;
    assert_eq!(form_page["items"].as_array().unwrap().len(), 1);
    assert_eq!(form_page["items"][0]["version"], 2);
    let requester_forms = request(&app, "/v1/forms", Some(&requester)).await.1;
    let requester_forms = requester_forms["items"].as_array().unwrap();
    assert!(
        requester_forms
            .iter()
            .all(|item| item["state"] == "PUBLISHED")
    );
    assert!(
        requester_forms
            .iter()
            .all(|item| item["id"].as_str() != Some(form_id.as_str()))
    );
    let (template_status, template, template_token) = create_template(
        &app,
        &policy_admin,
        "template-one",
        "Synthetic payment template",
        "00000000-0000-0000-0000-000000001a02",
        "PDF",
    )
    .await;
    assert_eq!(template_status, StatusCode::CREATED);
    assert_eq!(template["version"], 1);
    assert_eq!(template["state"], "DRAFT");
    let template_id = template["id"].as_str().unwrap().to_owned();
    assert_eq!(
        create_template(
            &app,
            &policy_admin,
            "template-one",
            "Synthetic payment template",
            "00000000-0000-0000-0000-000000001a02",
            "PDF",
        )
        .await
        .1,
        template
    );
    assert_eq!(
        create_template(
            &app,
            &policy_admin,
            "template-macro",
            "Macro",
            "00000000-0000-0000-0000-000000001a03",
            "DOCX",
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        create_template(
            &app,
            &requester,
            "template-denied",
            "Denied",
            "00000000-0000-0000-0000-000000001a02",
            "PDF",
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (template_revise_status, template_v2, template_v2_token) = revise_template(
        &app,
        &policy_admin,
        &template_id,
        "template-revise",
        template_token.as_deref().unwrap(),
        "00000000-0000-0000-0000-000000001a04",
        "New clean source",
    )
    .await;
    assert_eq!(template_revise_status, StatusCode::OK);
    assert_eq!(template_v2["version"], 2);
    assert_eq!(template_v2["id"], template["id"]);
    assert_ne!(template_token, template_v2_token);
    assert_eq!(
        revise_template(
            &app,
            &policy_admin,
            &template_id,
            "template-stale",
            template_token.as_deref().unwrap(),
            "00000000-0000-0000-0000-000000001a04",
            "stale",
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        create_template(
            &app,
            &policy_admin,
            "template-one",
            "Synthetic payment template",
            "00000000-0000-0000-0000-000000001a02",
            "PDF",
        )
        .await
        .1,
        template
    );
    assert_eq!(
        create_form(
            &app,
            &policy_admin,
            "form-one",
            "Synthetic payment form",
            fields.clone(),
        )
        .await
        .1,
        form
    );
    let (_, template_page) = request(&app, "/v1/templates", Some(&policy_admin)).await;
    assert_eq!(template_page["items"].as_array().unwrap().len(), 1);
    assert_eq!(template_page["items"][0]["version"], 2);
    let requester_templates = request(&app, "/v1/templates", Some(&requester)).await.1;
    let requester_templates = requester_templates["items"].as_array().unwrap();
    assert!(
        requester_templates
            .iter()
            .all(|item| item["state"] == "PUBLISHED")
    );
    assert!(
        requester_templates
            .iter()
            .all(|item| item["id"].as_str() != Some(template_id.as_str()))
    );
    let rule_definition = json!({"rule_id":"invoice_total_positive","version":1,"condition":{"op":"gt","args":[{"ref":"invoice.total"},{"literal":"0","value_type":"money","currency":"VND"}]},"severity":"BLOCK","waivable":false,"clause_id":"00000000-0000-0000-0000-000000001901"});
    let (rule_status, rule, rule_token) = create_rule(
        &app,
        &policy_admin,
        "rule-one",
        "invoice_total_positive",
        rule_definition.clone(),
    )
    .await;
    assert_eq!(rule_status, StatusCode::CREATED);
    assert_eq!(rule["version"], 1);
    assert_eq!(rule["severity"], "BLOCK");
    assert!(rule_token.is_some());
    let created_rule_id = rule["id"].as_str().unwrap().to_owned();
    assert_eq!(
        create_rule(
            &app,
            &policy_admin,
            "rule-one",
            "invoice_total_positive",
            rule_definition.clone()
        )
        .await
        .1,
        rule
    );
    let changed = json!({"rule_id":"invoice_total_positive","version":1,"condition":{"literal":false,"value_type":"boolean"},"severity":"BLOCK","waivable":false,"clause_id":"00000000-0000-0000-0000-000000001901"});
    assert_eq!(
        create_rule(
            &app,
            &policy_admin,
            "rule-one",
            "invoice_total_positive",
            changed
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        create_rule(
            &app,
            &requester,
            "rule-denied",
            "invoice_total_positive",
            rule_definition
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let mut revised_definition = rule["definition"].clone();
    revised_definition["version"] = json!(2);
    let revise_uri = format!("/v1/rules/{created_rule_id}/revise");
    let send_revision = |key: &str| {
        Request::builder()
            .method("POST")
            .uri(&revise_uri)
            .header(header::AUTHORIZATION, format!("Bearer {policy_admin}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header("idempotency-key", key)
            .header("if-finance-version", rule_token.as_deref().unwrap())
            .body(Body::from(
                json!({
                    "name":"invoice_total_positive",
                    "definition":revised_definition,
                    "reason":"Synthetic revision"
                })
                .to_string(),
            ))
            .unwrap()
    };
    let response = app
        .clone()
        .oneshot(send_revision("rule-revise"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let revised_rule: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(revised_rule["id"], rule["id"]);
    assert_eq!(revised_rule["version"], 2);
    let replay = app
        .clone()
        .oneshot(send_revision("rule-revise"))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
    let bytes = to_bytes(replay.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        revised_rule
    );
    let stale = app
        .clone()
        .oneshot(send_revision("rule-stale"))
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let simulate = |actor: &str, key: &str| {
        Request::builder()
            .method("POST")
            .uri(format!("/v1/rules/{created_rule_id}/simulate"))
            .header(header::AUTHORIZATION, format!("Bearer {actor}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header("idempotency-key", key)
            .header(
                "if-finance-version",
                format!("\"v1:rule:{created_rule_id}:2\""),
            )
            .body(Body::from(
                json!({"corpus_id":"00000000-0000-0000-0000-000000007500"}).to_string(),
            ))
            .unwrap()
    };
    let denied = app
        .clone()
        .oneshot(simulate(&requester, "simulation-denied"))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let result = app
        .clone()
        .oneshot(simulate(&policy_admin, "simulation-one"))
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let bytes = to_bytes(result.into_body(), 1024 * 1024).await.unwrap();
    let simulation: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(simulation["pass_count"], 1);
    assert_eq!(simulation["fail_count"], 1);
    assert_eq!(simulation["unknown_count"], 1);
    let replay = app
        .clone()
        .oneshot(simulate(&policy_admin, "simulation-one"))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
    let bytes = to_bytes(replay.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), simulation);
    let simulation_uri = format!("/v1/simulations/{}", simulation["id"].as_str().unwrap());
    assert_eq!(
        request(&app, &simulation_uri, Some(&policy_admin)).await.1,
        simulation
    );
    assert_eq!(
        request(&app, &simulation_uri, Some(&requester)).await.0,
        StatusCode::FORBIDDEN
    );
    let maker_rules = request(&app, "/v1/rules", Some(&policy_admin)).await.1;
    assert!(
        maker_rules["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"].as_str() == Some(created_rule_id.as_str()))
    );
    let requester_rules = request(&app, "/v1/rules", Some(&requester)).await.1;
    let requester_rules = requester_rules["items"].as_array().unwrap();
    assert_eq!(requester_rules.len(), 1);
    assert_eq!(
        requester_rules[0]["id"],
        "00000000-0000-0000-0000-000000002103"
    );
    assert!(
        requester_rules
            .iter()
            .all(|item| item["id"].as_str() != Some(created_rule_id.as_str()))
    );
    let waiver_dossier = "00000000-0000-0000-0000-000000002120";
    // Exercise the SQL security boundary directly using the runtime role.
    // Even with a valid same-company dossier/owner argument, the session actor
    // must not be allowed to impersonate that owner.
    let mut checklist_tx = runtime.begin().await.unwrap();
    sqlx::query("SELECT set_config('finance.tenant_id',$1,true),set_config('finance.company_id',$2,true),set_config('finance.actor_id',$3,true)")
        .bind(id(1).to_string())
        .bind(id(10).to_string())
        .bind(id(1002).to_string())
        .execute(&mut *checklist_tx)
        .await
        .unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM finance.runtime_checklist($1,$2,$3,$4,$5,$6)")
            .bind(id(1))
            .bind(id(10))
            .bind(id(1001))
            .bind(Uuid::parse_str(waiver_dossier).unwrap())
            .bind(Uuid::nil())
            .bind(10_i32)
            .fetch_one(&mut *checklist_tx)
            .await
            .unwrap();
    assert_eq!(count, 0);
    checklist_tx.rollback().await.unwrap();
    let (checklist_status, checklist) = request(
        &app,
        &format!("/v1/dossiers/{waiver_dossier}/checklist?limit=10"),
        Some(&requester),
    )
    .await;
    assert_eq!(checklist_status, StatusCode::OK);
    assert_eq!(checklist["items"].as_array().unwrap().len(), 1);
    assert_eq!(checklist["items"][0]["requirement"], "INVOICE");
    assert_eq!(checklist["items"][0]["state"], "MISSING");
    assert_eq!(
        checklist["items"][0]["clause_id"],
        "00000000-0000-0000-0000-000000002101"
    );
    assert_eq!(checklist["items"][0]["evidence_ids"], json!([]));
    assert_eq!(
        request(
            &app,
            &format!("/v1/dossiers/{waiver_dossier}/checklist?limit=10"),
            Some(&other_requester),
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let waiver_version = format!("\"v1:dossier:{waiver_dossier}:0\"");
    let (waiver_status, waiver, waiver_receipt) = request_waiver(
        &app,
        &requester,
        waiver_dossier,
        "waiver-one",
        &waiver_version,
        "00000000-0000-0000-0000-000000002123",
    )
    .await;
    assert_eq!(waiver_status, StatusCode::OK);
    assert_eq!(waiver["state"], "REQUESTED");
    assert_eq!(waiver_receipt.as_deref(), Some(waiver_version.as_str()));
    let waiver_id = waiver["id"].as_str().unwrap().to_owned();
    let waiver_expiry = chrono::DateTime::from_timestamp((now() + 86_400) as i64, 0)
        .unwrap()
        .to_rfc3339();
    let approver = mint("form-checker");
    assert_eq!(
        decide_waiver(
            &app,
            &requester,
            waiver_dossier,
            &waiver_id,
            "waiver-self",
            &waiver_version,
            ("APPROVED", Some(waiver_expiry.as_str()))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (decision_status, decision, _) = decide_waiver(
        &app,
        &approver,
        waiver_dossier,
        &waiver_id,
        "waiver-approve",
        &waiver_version,
        ("APPROVED", Some(waiver_expiry.as_str())),
    )
    .await;
    assert_eq!(decision_status, StatusCode::OK);
    assert_eq!(decision["state"], "APPROVED");
    assert_eq!(
        request(
            &app,
            &format!("/v1/dossiers/{waiver_dossier}/waivers"),
            Some(&requester)
        )
        .await
        .1["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let handoff_dossier = "00000000-0000-0000-0000-000000002300";
    let handoff_version = format!("\"v1:dossier:{handoff_dossier}:0\"");
    let (handoff_status, handoff, handoff_receipt) = create_handoff(
        &app,
        &requester,
        handoff_dossier,
        "handoff-one",
        &handoff_version,
        "00000000-0000-0000-0000-000000002304",
        ("MANUAL", Some("MANUAL-RECEIPT-1")),
    )
    .await;
    assert_eq!(handoff_status, StatusCode::CREATED);
    assert_eq!(handoff["state"], "ACKNOWLEDGED");
    assert_ne!(handoff["state"], "APPROVED");
    let payment_uri = format!("/v1/dossiers/{handoff_dossier}/payment-status");
    let (payment_status, payment) = request(&app, &payment_uri, Some(&requester)).await;
    assert_eq!(payment_status, StatusCode::OK);
    assert_eq!(payment["state"], "UNKNOWN");
    assert!(payment.get("confirmed_paid").is_none());
    assert_eq!(
        request(&app, &payment_uri, Some(&other_requester)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, &payment_uri, Some(&auditor)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&app, &payment_uri, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert!(handoff_receipt.as_deref().unwrap().ends_with(":1\""));
    assert_eq!(
        create_handoff(
            &app,
            &requester,
            handoff_dossier,
            "handoff-do",
            &handoff_version,
            "00000000-0000-0000-0000-000000002304",
            ("DO_API", None)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(
            &app,
            &format!("/v1/dossiers/{handoff_dossier}/handoffs"),
            Some(&requester)
        )
        .await
        .1["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let recon_id = "00000000-0000-0000-0000-000000002316";
    let (recon_status, recon, recon_token) = request_with_headers(
        &app,
        &format!("/v1/reconciliation/{recon_id}"),
        Some(&requester),
    )
    .await;
    assert_eq!(recon_status, StatusCode::OK);
    assert_eq!(recon["state"], "OPEN");
    assert!(recon_token.as_deref().unwrap().ends_with(":0\""));
    let approver = mint("form-checker");
    assert_eq!(
        resolve_reconciliation(
            &app,
            &requester,
            recon_id,
            "recon-denied",
            recon_token.as_deref().unwrap()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (resolve_status, resolved, _) = resolve_reconciliation(
        &app,
        &approver,
        recon_id,
        "recon-resolve",
        recon_token.as_deref().unwrap(),
    )
    .await;
    assert_eq!(resolve_status, StatusCode::OK);
    assert_eq!(resolved["state"], "RESOLVED");

    let review_dossier = "00000000-0000-0000-0000-000000002320";
    let review_snapshot = "00000000-0000-0000-0000-000000002323";
    let risk_id = "00000000-0000-0000-0000-000000002334";
    let risk_v0 = format!("\"v1:risk:{risk_id}:0\"");
    let (risk_status, open_risk) =
        request(&app, &format!("/v1/risks/{risk_id}"), Some(&requester)).await;
    assert_eq!(risk_status, StatusCode::OK);
    assert_eq!(open_risk["state"], "OPEN");
    assert_eq!(
        resolve_risk(&app, &requester, risk_id, "risk-owner-denied", &risk_v0)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (risk_resolve_status, resolved_risk, risk_receipt) =
        resolve_risk(&app, &review_accountant, risk_id, "risk-resolve", &risk_v0).await;
    assert_eq!(risk_resolve_status, StatusCode::OK);
    assert_eq!(resolved_risk["state"], "RESOLVED");
    assert!(risk_receipt.as_deref().unwrap().ends_with(":1\""));
    let risk_replay =
        resolve_risk(&app, &review_accountant, risk_id, "risk-resolve", &risk_v0).await;
    assert_eq!(risk_replay.0, StatusCode::OK);
    assert_eq!(risk_replay.1, resolved_risk);
    assert_eq!(risk_replay.2, risk_receipt);
    assert_eq!(
        resolve_risk(&app, &review_accountant, risk_id, "risk-stale", &risk_v0,)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(&app, &format!("/v1/risks/{risk_id}"), Some(&requester))
            .await
            .1["state"],
        "RESOLVED"
    );
    let review_v0 = format!("\"v1:dossier:{review_dossier}:0\"");
    let (queue_status, queue) =
        request(&app, "/v1/review/queue?limit=10", Some(&review_accountant)).await;
    assert_eq!(queue_status, StatusCode::OK);
    assert!(
        queue["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == review_dossier)
    );
    assert_eq!(
        begin_review(
            &app,
            &requester,
            review_dossier,
            "review-owner-denied",
            &review_v0,
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (begin_status, begun, begin_token) = begin_review(
        &app,
        &review_accountant,
        review_dossier,
        "review-begin",
        &review_v0,
    )
    .await;
    assert_eq!(begin_status, StatusCode::OK);
    assert_eq!(begun["status"], "ACCOUNTING_REVIEW");
    assert_eq!(begun["version"], 1);
    assert!(begin_token.as_deref().unwrap().ends_with(":1\""));
    let (read_status, pending) = request(
        &app,
        &format!("/v1/dossiers/{review_dossier}/review"),
        Some(&requester),
    )
    .await;
    assert_eq!(read_status, StatusCode::OK);
    assert_eq!(pending["decision"], "PENDING");
    assert_eq!(pending["snapshot_id"], review_snapshot);
    assert_eq!(
        request(
            &app,
            &format!("/v1/dossiers/{review_dossier}/review"),
            Some(&other_requester)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (accept_status, accepted, accept_token) = decide_review(
        &app,
        &review_accountant,
        review_dossier,
        "review-accept",
        begin_token.as_deref().unwrap(),
        review_snapshot,
        "ACCEPTED",
    )
    .await;
    assert_eq!(accept_status, StatusCode::OK);
    assert_eq!(accepted["decision"], "ACCEPTED");
    assert_eq!(accepted["confirmed_field_ids"], json!([]));
    assert!(accept_token.as_deref().unwrap().ends_with(":2\""));
    assert_eq!(
        decide_review(
            &app,
            &review_accountant,
            review_dossier,
            "review-accept",
            begin_token.as_deref().unwrap(),
            review_snapshot,
            "ACCEPTED",
        )
        .await
        .1,
        accepted
    );
    assert_eq!(
        decide_review(
            &app,
            &review_accountant,
            review_dossier,
            "review-stale",
            begin_token.as_deref().unwrap(),
            review_snapshot,
            "ACCEPTED",
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let queue_after = request(&app, "/v1/review/queue?limit=10", Some(&review_accountant))
        .await
        .1;
    assert!(
        !queue_after["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == review_dossier)
    );

    let ready_id = "00000000-0000-0000-0000-000000001800";
    let check_id = "00000000-0000-0000-0000-000000001802";
    let ready_version = format!("\"v1:dossier:{ready_id}:0\"");
    let (check_status, check_page) = request(
        &app,
        &format!("/v1/dossiers/{ready_id}/checks?limit=100"),
        Some(&requester),
    )
    .await;
    assert_eq!(check_status, StatusCode::OK);
    let current_checks = check_page["items"].as_array().unwrap();
    assert_eq!(current_checks.len(), 2);
    let pass = current_checks
        .iter()
        .find(|x| x["reason_code"] == "SYNTHETIC_PASS")
        .unwrap();
    let warning = current_checks
        .iter()
        .find(|x| x["severity"] == "WARNING")
        .unwrap();
    let ack_uri = format!("/v1/dossiers/{ready_id}/acknowledge");
    let ack_request = |actor: &str, key: &str, result: &Value| {
        Request::builder()
            .method("POST")
            .uri(ack_uri.as_str())
            .header(header::AUTHORIZATION, format!("Bearer {actor}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header("idempotency-key", key)
            .header("if-finance-version", ready_version.as_str())
            .body(Body::from(
                json!({"result_id":result,"reason":"Reviewed warning"}).to_string(),
            ))
            .unwrap()
    };
    let refused = app
        .clone()
        .oneshot(ack_request(&requester, "ack-block", &pass["id"]))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(
        ready_dossier(
            &app,
            &requester,
            ready_id,
            "ready-unacknowledged-warning",
            &ready_version,
            check_id,
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let acknowledged = app
        .clone()
        .oneshot(ack_request(&requester, "ack-warning", &warning["id"]))
        .await
        .unwrap();
    assert_eq!(acknowledged.status(), StatusCode::OK);
    let body = to_bytes(acknowledged.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(&serde_json::from_slice::<Value>(&body).unwrap(), warning);
    let replay = app
        .clone()
        .oneshot(ack_request(&requester, "ack-warning", &warning["id"]))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
    let bytes = to_bytes(replay.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(body, bytes);
    assert_eq!(
        ready_dossier(
            &app,
            &requester,
            ready_id,
            "ready-old-check",
            &ready_version,
            "00000000-0000-0000-0000-000000001810",
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (ready_status, ready_body, ready_receipt) = ready_dossier(
        &app,
        &requester,
        ready_id,
        "ready-one",
        &ready_version,
        check_id,
    )
    .await;
    assert_eq!(ready_status, StatusCode::OK);
    assert_eq!(ready_body["status"], "READY");
    assert_eq!(ready_body["version"], 1);
    assert_eq!(
        ready_dossier(
            &app,
            &requester,
            ready_id,
            "ready-one",
            &ready_version,
            check_id,
        )
        .await
        .1,
        ready_body
    );
    assert_eq!(
        ready_dossier(
            &app,
            &requester,
            ready_id,
            "ready-stale",
            &ready_version,
            check_id,
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (snapshot_status, sealed) = request(
        &app,
        &format!("/v1/dossiers/{ready_id}/snapshot"),
        Some(&requester),
    )
    .await;
    assert_eq!(snapshot_status, StatusCode::OK);
    assert_eq!(sealed["dossier_id"], ready_id);
    assert_eq!(sealed["check_run_id"], check_id);
    assert_eq!(
        sealed["policy_version_id"],
        "00000000-0000-0000-0000-000000001600"
    );
    assert_eq!(sealed["revision"], 0);
    assert_eq!(sealed["input_hash"].as_str().unwrap().len(), 64);
    assert!(ready_receipt.as_deref().unwrap().ends_with(":1\""));
    assert_ne!(ready_receipt.as_deref().unwrap(), ready_version);
    let snapshot_id = sealed["id"].as_str().unwrap();
    let (export_status, export_job, export_receipt) = create_export(
        &app,
        &requester,
        ready_id,
        "export-one",
        ready_receipt.as_deref().unwrap(),
        snapshot_id,
        "00000000-0000-0000-0000-000000001804",
    )
    .await;
    assert_eq!(export_status, StatusCode::ACCEPTED);
    assert_eq!(export_job["state"], "QUEUED");
    assert_eq!(export_job["attempt"], 0);
    assert_eq!(export_job["progress"], 0);
    assert_eq!(export_receipt, ready_receipt);
    let (packages_status, packages) = request(
        &app,
        &format!("/v1/dossiers/{ready_id}/exports?limit=10"),
        Some(&requester),
    )
    .await;
    assert_eq!(packages_status, StatusCode::OK);
    assert_eq!(packages["items"].as_array().unwrap().len(), 1);
    assert_eq!(packages["items"][0]["state"], "QUEUED");
    assert_eq!(packages["items"][0]["snapshot_id"], snapshot_id);
    assert_eq!(
        request(
            &app,
            &format!("/v1/dossiers/{ready_id}/exports?limit=10"),
            Some(&other_requester),
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let job_uri = format!("/v1/jobs/{}", export_job["id"].as_str().unwrap());
    assert_eq!(
        request(&app, &job_uri, Some(&requester)).await,
        (StatusCode::OK, export_job.clone())
    );
    assert_eq!(
        request(&app, &job_uri, Some(&other_requester)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, &job_uri, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        create_export(
            &app,
            &requester,
            ready_id,
            "export-one",
            ready_receipt.as_deref().unwrap(),
            snapshot_id,
            "00000000-0000-0000-0000-000000001804",
        )
        .await
        .1,
        export_job
    );
    assert_eq!(
        request(
            &app,
            &format!("/v1/dossiers/{ready_id}/snapshot"),
            Some(&other_requester),
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let uri = "/v1/organizations?limit=1";
    assert_eq!(request(&app, uri, None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        request(&app, uri, Some("invalid")).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, uri, Some(&auditor)).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, first) = request(&app, uri, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    let cursor = first["next_cursor"].as_str().unwrap();
    let (status, second) = request(&app, &format!("{uri}&cursor={cursor}"), Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(first["items"][0]["id"], second["items"][0]["id"]);
    assert_eq!(
        request(&app, &format!("{uri}&cursor={cursor}x"), Some(&token))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(&app, "/v1/organizations?limit=101", Some(&token))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &app,
            "/v1/organizations?company_id=00000000-0000-0000-0000-00000000000b",
            Some(&token)
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    // Exercise independent requests without privileged DB access.
    for _ in 0..8 {
        let (a, b) = tokio::join!(
            request(&app, uri, Some(&token)),
            request(&app, uri, Some(&auditor))
        );
        assert_eq!(a.0, StatusCode::OK);
        assert_eq!(b.0, StatusCode::FORBIDDEN);
    }

    // Three dossier types, exact idempotency under concurrency and owner isolation.
    let (left, right) = tokio::join!(
        create_dossier(&app, &requester, "pay-one", "PAYMENT"),
        create_dossier(&app, &requester, "pay-one", "PAYMENT")
    );
    assert_eq!(left.0, StatusCode::CREATED);
    assert_eq!(right.0, StatusCode::CREATED);
    assert_eq!(left.1, right.1);
    assert_eq!(left.1["status"], "DRAFT");
    assert_eq!(left.1["revision"], 0);
    assert_eq!(left.1["version"], 0);
    let dossier_uri = format!("/v1/dossiers/{}", left.1["id"].as_str().unwrap());
    assert_eq!(
        request(&app, &dossier_uri, Some(&requester)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        request(&app, &dossier_uri, Some(&other_requester)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        create_dossier(&app, &requester, "pay-one", "ADVANCE")
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        create_dossier(&app, &auditor, "denied", "PAYMENT").await.0,
        StatusCode::FORBIDDEN
    );
    let (advance_status, advance) =
        create_dossier(&app, &requester, "advance-one", "ADVANCE").await;
    assert_eq!(advance_status, StatusCode::CREATED);
    let (settlement_status, settlement) =
        create_dossier(&app, &requester, "settlement-one", "SETTLEMENT").await;
    assert_eq!(settlement_status, StatusCode::CREATED);
    let (status, page) = request(&app, "/v1/dossiers", Some(&requester)).await;
    assert_eq!(status, StatusCode::OK);
    let dossier_items = page["items"].as_array().unwrap();
    for id in [
        left.1["id"].as_str().unwrap(),
        advance["id"].as_str().unwrap(),
        settlement["id"].as_str().unwrap(),
    ] {
        assert!(
            dossier_items
                .iter()
                .any(|item| item["id"].as_str() == Some(id))
        );
    }
    let change_id = settlement["id"].as_str().unwrap();
    let change_request = |op: &str, key: &str, version: u64, actor: &str| {
        Request::builder()
            .method("POST")
            .uri(format!("/v1/dossiers/{change_id}/{op}"))
            .header(header::AUTHORIZATION, format!("Bearer {actor}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header("idempotency-key", key)
            .header(
                "if-finance-version",
                format!("\"v1:dossier:{change_id}:{version}\""),
            )
            .body(Body::from(
                json!({"reason":"Synthetic lifecycle test","evidence_ids":[]}).to_string(),
            ))
            .unwrap()
    };
    let denied = app
        .clone()
        .oneshot(change_request(
            "revise",
            "revise-denied",
            0,
            &other_requester,
        ))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let invalid = app
        .clone()
        .oneshot(change_request("reopen", "reopen-draft", 0, &requester))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::CONFLICT);
    let revised = app
        .clone()
        .oneshot(change_request("revise", "revise-settlement", 0, &requester))
        .await
        .unwrap();
    assert_eq!(revised.status(), StatusCode::OK);
    let body = to_bytes(revised.into_body(), 1024 * 1024).await.unwrap();
    let revised: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(revised["version"], 1);
    assert_eq!(revised["revision"], 1);
    let cancelled = app
        .clone()
        .oneshot(change_request("cancel", "cancel-settlement", 1, &requester))
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::OK);
    let bytes = to_bytes(cancelled.into_body(), 1024 * 1024).await.unwrap();
    let cancelled: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(cancelled["status"], "CANCELLED");
    assert_eq!(cancelled["version"], 2);
    let replay = app
        .clone()
        .oneshot(change_request("revise", "revise-settlement", 0, &requester))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
    let bytes = to_bytes(replay.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), revised);
    let (dashboard_status, dashboard) = request(&app, "/v1/dashboard", Some(&requester)).await;
    assert_eq!(dashboard_status, StatusCode::OK);
    let total = dashboard["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == "dossiers.total")
        .unwrap();
    assert_eq!(
        total["value"].as_str().unwrap().parse::<usize>().unwrap(),
        dossier_items.len()
    );
    assert_eq!(
        request(&app, "/v1/dashboard", Some(&auditor)).await.0,
        StatusCode::FORBIDDEN
    );

    let (status, page) = request(
        &app,
        "/v1/dossiers?kind=ADVANCE&status=DRAFT",
        Some(&requester),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["kind"], "ADVANCE");
    let (_, empty) = request(
        &app,
        "/v1/dossiers?updated_to=2000-01-01T00%3A00%3A00Z",
        Some(&requester),
    )
    .await;
    assert_eq!(empty["items"].as_array().unwrap().len(), 0);
    assert_eq!(
        request(&app, "/v1/dossiers?kind=INVALID", Some(&requester))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    let (_, first) = request(&app, "/v1/dossiers?limit=1", Some(&requester)).await;
    let cursor = first["next_cursor"].as_str().unwrap();
    assert_eq!(
        request(
            &app,
            &format!("/v1/dossiers?limit=1&kind=ADVANCE&cursor={cursor}"),
            Some(&requester),
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &app,
            "/v1/dossiers?company_id=00000000-0000-0000-0000-00000000000b",
            Some(&requester),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (_, page) = request(&app, "/v1/dossiers", Some(&other_requester)).await;
    assert_eq!(page["items"].as_array().unwrap().len(), 0);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(&dossier_uri)
                .header(header::AUTHORIZATION, format!("Bearer {requester}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"status":"ACCEPTED"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);

    let checks_uri = format!("{dossier_uri}/checks");
    let (status, page) = request(&app, &checks_uri, Some(&requester)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["items"], json!([]));
    assert_eq!(
        request(&app, &checks_uri, Some(&other_requester)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, &checks_uri, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, &checks_uri, Some(&auditor)).await.0,
        StatusCode::FORBIDDEN
    );
    let doc_uri = format!("{dossier_uri}/documents");
    let draft_id = left.1["id"].as_str().unwrap();
    let create = |key: &'static str, version, size| {
        create_document(&app, &requester, &doc_uri, draft_id, key, version, size)
    };
    let (a, b) = tokio::join!(
        create("doc-one", 0, 26214400),
        create("doc-one", 0, 26214400)
    );
    assert_eq!(a.0, StatusCode::CREATED);
    assert_eq!(a, b);
    assert_eq!(create("doc-one", 0, 1).await.0, StatusCode::CONFLICT);
    assert_eq!(create("stale", 0, 1).await.0, StatusCode::CONFLICT);
    assert_eq!(
        create("oversize", 1, 26214401).await.0,
        StatusCode::BAD_REQUEST
    );
    for version in 1..4 {
        let key = format!("doc-{version}");
        assert_eq!(
            create_document(
                &app, &requester, &doc_uri, draft_id, &key, version, 26214400
            )
            .await
            .0,
            StatusCode::CREATED
        );
    }
    assert_eq!(create("overquota", 4, 1).await.0, StatusCode::CONFLICT);
    assert_eq!(create("doc-one", 0, 26214400).await, a);
    assert_eq!(
        create_dossier(&app, &requester, "pay-one", "PAYMENT").await,
        left
    );
    let preflight_uri = format!("/v1/dossiers/{draft_id}/checks");
    let preflight = |actor: &str, version: u64, key: &str| {
        Request::builder()
            .method("POST")
            .uri(preflight_uri.as_str())
            .header(header::AUTHORIZATION, format!("Bearer {actor}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header("idempotency-key", key)
            .header(
                "if-finance-version",
                format!("\"v1:dossier:{draft_id}:{version}\""),
            )
            .body(Body::from(
                json!({"policy_version_id":policy_id}).to_string(),
            ))
            .unwrap()
    };
    let denied = app
        .clone()
        .oneshot(preflight(&other_requester, 4, "preflight-denied"))
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let stale = app
        .clone()
        .oneshot(preflight(&requester, 3, "preflight-stale"))
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let (_, updated) = request(&app, &dossier_uri, Some(&requester)).await;
    assert_eq!(updated["version"], 4);
    let (documents_status, documents) = request(&app, &doc_uri, Some(&requester)).await;
    assert_eq!(documents_status, StatusCode::OK);
    assert_eq!(documents["items"].as_array().unwrap().len(), 4);
    assert_eq!(documents["items"][0]["state"], "PENDING_UPLOAD");
    assert_eq!(
        request(&app, &doc_uri, Some(&other_requester)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        create_document(&app, &other_requester, &doc_uri, draft_id, "other", 4, 1)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(&app, &doc_uri, Some(&auditor)).await.0,
        StatusCode::FORBIDDEN
    );

    if let Ok(python) = std::env::var("FINANCE_BROWSER_PYTHON") {
        let root = PathBuf::from(std::env::var("FINANCE_BROWSER_DIST").unwrap());
        let static_router = Router::new()
            .route("/", get(index))
            .route("/assets/{name}", get(asset))
            .with_state(root);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app.merge(static_router))
                .await
                .unwrap();
        });
        let script = std::env::var("FINANCE_BROWSER_SCRIPT").unwrap();
        let result = tokio::task::spawn_blocking(move || {
            std::process::Command::new(python)
                .arg(script)
                .env("FINANCE_TEST_ORIGIN", format!("http://{address}"))
                .env("FINANCE_TEST_TOKEN", token)
                .env("FINANCE_TEST_AUDITOR", auditor)
                .env("FINANCE_TEST_REQUESTER", requester)
                .status()
                .unwrap()
        })
        .await
        .unwrap();
        server.abort();
        assert!(result.success(), "synthetic browser integration failed");
    }
    // Boot the actual binary separately from the injected test router.
    let jwks_for_binary = jwks.clone();
    let requester_for_binary = mint("requester");
    tokio::task::spawn_blocking(move || binary_probe(&jwks_for_binary, &requester_for_binary))
        .await
        .unwrap();
    runtime.close().await;
    lookup.close().await;
}

async fn create_form(
    app: &Router,
    token: &str,
    key: &str,
    name: &str,
    fields: Value,
) -> (StatusCode, Value, Option<String>) {
    let body =
        json!({"policy_id":"00000000-0000-0000-0000-000000001900","name":name,"fields":fields});
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/forms")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", key)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    (status, value, receipt)
}

async fn revise_form(
    app: &Router,
    token: &str,
    form: &str,
    key: &str,
    version: &str,
    revision: (&str, Value, &str),
) -> (StatusCode, Value, Option<String>) {
    let (name, fields, reason) = revision;
    let body = json!({"name":name,"fields":fields,"reason":reason});
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/forms/{form}/revise"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", key)
                .header("if-finance-version", version)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    (status, value, receipt)
}

async fn create_workflow(
    app: &Router,
    token: &str,
    key: &str,
    name: &str,
    transitions: Value,
) -> (StatusCode, Value, Option<String>) {
    let body = json!({
        "policy_id":"00000000-0000-0000-0000-000000001900",
        "name":name,
        "transitions":transitions
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/workflows")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", key)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    (status, value, receipt)
}

async fn revise_workflow(
    app: &Router,
    token: &str,
    workflow: &str,
    key: &str,
    version: &str,
    revision: (&str, Value, &str),
) -> (StatusCode, Value, Option<String>) {
    let (name, transitions, reason) = revision;
    let body = json!({"name":name,"transitions":transitions,"reason":reason});
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/workflows/{workflow}/revise"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", key)
                .header("if-finance-version", version)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    (status, value, receipt)
}

async fn create_template(
    app: &Router,
    token: &str,
    key: &str,
    name: &str,
    document_id: &str,
    format: &str,
) -> (StatusCode, Value, Option<String>) {
    let body = json!({
        "policy_id":"00000000-0000-0000-0000-000000001900",
        "name":name,
        "document_id":document_id,
        "format":format
    });
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/templates")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", key)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .map(|value| value.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    (status, value, receipt)
}

async fn revise_template(
    app: &Router,
    token: &str,
    template: &str,
    key: &str,
    version: &str,
    document_id: &str,
    reason: &str,
) -> (StatusCode, Value, Option<String>) {
    let body = json!({"document_id":document_id,"reason":reason});
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/templates/{template}/revise"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", key)
                .header("if-finance-version", version)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .map(|value| value.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    (status, value, receipt)
}

async fn ready_dossier(
    app: &Router,
    token: &str,
    dossier: &str,
    key: &str,
    version: &str,
    check_run: &str,
) -> (StatusCode, Value, Option<String>) {
    let body = json!({"check_run_id":check_run});
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/dossiers/{dossier}/ready"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", key)
                .header("if-finance-version", version)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .map(|value| value.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    (status, value, receipt)
}

async fn create_dossier(app: &Router, token: &str, key: &str, kind: &str) -> (StatusCode, Value) {
    let body = json!({"company_id":id(10),"kind":kind});
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/dossiers")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", key)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let version = response.headers().get("finance-version").cloned();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    if status == StatusCode::CREATED {
        assert_eq!(
            version.unwrap().to_str().unwrap(),
            format!("\"v1:dossier:{}:0\"", value["id"].as_str().unwrap())
        );
    }
    (status, value)
}

async fn create_document(
    app: &Router,
    token: &str,
    uri: &str,
    dossier: &str,
    key: &str,
    version: i64,
    size: u64,
) -> (StatusCode, Value, Option<String>) {
    let body = json!({"file_name":"synthetic.pdf","mime":"application/pdf","bytes":size,"sha256":"a".repeat(64),"kind":"INVOICE"});
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("idempotency-key", key)
                .header(
                    "if-finance-version",
                    format!("\"v1:dossier:{dossier}:{version}\""),
                )
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    if status == StatusCode::CREATED {
        assert_eq!(
            receipt.as_deref(),
            Some(format!("\"v1:dossier:{dossier}:{}\"", version + 1).as_str())
        );
    }
    (status, value, receipt)
}

fn binary_probe(jwks: &str, token: &str) {
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        process::{Child, Command, Stdio},
        time::Duration,
    };
    struct Guard(Child);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let address = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut process = Command::new(env!("CARGO_BIN_EXE_finance-api"));
    process
        .arg("--synthetic")
        .env("FINANCE_BIND", address.to_string())
        .env(
            "FINANCE_SYNTHETIC_RUNTIME_DATABASE_URL",
            std::env::var("FINANCE_PROTOTYPE_TEST_DATABASE_URL").unwrap(),
        )
        .env(
            "FINANCE_SYNTHETIC_LOOKUP_DATABASE_URL",
            std::env::var("FINANCE_IDENTITY_TEST_DATABASE_URL").unwrap(),
        )
        .env("FINANCE_SYNTHETIC_JWKS", jwks)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = Guard(process.spawn().unwrap());
    let mut ready = false;
    for _ in 0..100 {
        if child.0.try_wait().unwrap().is_some() {
            panic!("synthetic API exited before readiness");
        }
        if let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(100)) {
            stream
                .set_read_timeout(Some(Duration::from_secs(4)))
                .unwrap();
            stream
                .write_all(
                    b"GET /health/ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            let mut response = String::new();
            stream.take(8192).read_to_string(&mut response).unwrap();
            if response.starts_with("HTTP/1.1 200 ") {
                ready = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(ready, "synthetic API did not become ready");
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(stream,"GET /v1/dossiers?limit=1 HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.take(65536).read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200 "));
    let body = response.split_once("\r\n\r\n").unwrap().1;
    let page: Value = serde_json::from_str(body).unwrap();
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["id"].as_str().is_some());
    assert!(items[0]["kind"].as_str().is_some());
    assert!(items[0]["status"].as_str().is_some());
    drop(child);
    let denied = process.env("FINANCE_BIND", "0.0.0.0:0").status().unwrap();
    assert_eq!(denied.code(), Some(78));
}

async fn create_rule(
    app: &Router,
    token: &str,
    key: &str,
    name: &str,
    definition: serde_json::Value,
) -> (StatusCode, serde_json::Value, Option<String>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/rules")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("idempotency-key", key)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "policy_id":"00000000-0000-0000-0000-000000001900",
                        "name":name,
                        "definition":definition
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let version = response
        .headers()
        .get("finance-version")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
    (status, body, version)
}

async fn request_waiver(
    app: &Router,
    token: &str,
    dossier: &str,
    key: &str,
    version: &str,
    result: &str,
) -> (StatusCode, serde_json::Value, Option<String>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/dossiers/{dossier}/waivers"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("idempotency-key", key)
                .header("if-finance-version", version)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "result_id":result,
                        "reason":"Synthetic waiver request",
                        "evidence_ids":[]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({})),
        receipt,
    )
}

async fn decide_waiver(
    app: &Router,
    token: &str,
    dossier: &str,
    waiver: &str,
    key: &str,
    version: &str,
    outcome: (&str, Option<&str>),
) -> (StatusCode, serde_json::Value, Option<String>) {
    let (decision, expires) = outcome;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/dossiers/{dossier}/waivers/{waiver}/decision"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("idempotency-key", key)
                .header("if-finance-version", version)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "decision":decision,
                        "expires_at":expires,
                        "reason":"Synthetic independent decision",
                        "evidence_ids":[]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({})),
        receipt,
    )
}

async fn request_with_headers(
    app: &Router,
    uri: &str,
    token: Option<&str>,
) -> (StatusCode, serde_json::Value, Option<String>) {
    let mut builder = Request::builder().uri(uri);
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let version = response
        .headers()
        .get("finance-version")
        .and_then(|x| x.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({})),
        version,
    )
}

async fn create_handoff(
    app: &Router,
    token: &str,
    dossier: &str,
    key: &str,
    version: &str,
    package: &str,
    channel_receipt: (&str, Option<&str>),
) -> (StatusCode, serde_json::Value, Option<String>) {
    let (channel, external) = channel_receipt;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/dossiers/{dossier}/handoffs"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("idempotency-key", key)
                .header("if-finance-version", version)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "package_id":package,
                        "channel":channel,
                        "external_ref":external,
                        "evidence_ids":[]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .and_then(|x| x.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({})),
        receipt,
    )
}

async fn resolve_reconciliation(
    app: &Router,
    token: &str,
    id: &str,
    key: &str,
    version: &str,
) -> (StatusCode, serde_json::Value, Option<String>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/reconciliation/{id}/resolve"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("idempotency-key", key)
                .header("if-finance-version", version)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "reason":"Authoritative manual reconciliation completed",
                        "evidence_ids":[]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .and_then(|x| x.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({})),
        receipt,
    )
}

async fn create_export(
    app: &Router,
    token: &str,
    dossier: &str,
    key: &str,
    version: &str,
    snapshot: &str,
    template: &str,
) -> (StatusCode, serde_json::Value, Option<String>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/dossiers/{dossier}/exports"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("idempotency-key", key)
                .header("if-finance-version", version)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"snapshot_id":snapshot,"template_ids":[template]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({})),
        receipt,
    )
}

async fn begin_review(
    app: &Router,
    token: &str,
    dossier: &str,
    key: &str,
    version: &str,
) -> (StatusCode, Value, Option<String>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/dossiers/{dossier}/begin-review"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("idempotency-key", key)
                .header("if-finance-version", version)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"reason":"Independent accounting review","evidence_ids":[]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({})),
        receipt,
    )
}

async fn decide_review(
    app: &Router,
    token: &str,
    dossier: &str,
    key: &str,
    version: &str,
    snapshot: &str,
    decision: &str,
) -> (StatusCode, Value, Option<String>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/dossiers/{dossier}/review"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("idempotency-key", key)
                .header("if-finance-version", version)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "snapshot_id":snapshot,
                        "decision":decision,
                        "confirmed_field_ids":[],
                        "reason":"All configured accounting review controls satisfied"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let receipt = response
        .headers()
        .get("finance-version")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({})),
        receipt,
    )
}

async fn create_membership(
    app: &Router,
    token: &str,
    key: &str,
    subject: &str,
    roles: &[&str],
) -> (StatusCode, Value, Option<String>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/memberships")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("idempotency-key", key)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "subject_id": subject,
                        "company_id": "00000000-0000-0000-0000-00000000000a",
                        "roles": roles,
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let version = response
        .headers()
        .get("finance-version")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap(), version)
}

async fn revoke_membership(
    app: &Router,
    token: &str,
    subject: &str,
    key: &str,
    version: &str,
) -> (StatusCode, Value, Option<String>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/memberships/{subject}/revoke"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header("idempotency-key", key)
                .header("if-finance-version", version)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({"reason":"synthetic revocation"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let next = response
        .headers()
        .get("finance-version")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap(), next)
}

async fn post_contract(
    app: &Router,
    token: &str,
    uri: &str,
    key: &str,
    version: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, Option<String>) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header("idempotency-key", key);
    if let Some(version) = version {
        builder = builder.header("if-finance-version", version);
    }
    let req = if let Some(payload) = body {
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(payload.to_string()))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status();
    let next = response
        .headers()
        .get("finance-version")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let parsed = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, parsed, next)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires disposable PostgreSQL and synthetic test credentials"]
async fn complete_api_contracts_real_database() {
    let runtime = PgPoolOptions::new()
        .max_connections(3)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE finance_runtime")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&std::env::var("FINANCE_PROTOTYPE_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let lookup = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|conn, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE finance_identity_lookup")
                    .execute(conn)
                    .await?;
                Ok(())
            })
        })
        .connect(&std::env::var("FINANCE_IDENTITY_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let key = RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).unwrap();
    let jwks = json!({"keys":[{"kty":"RSA","kid":"http-test","alg":"RS256","n":URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()),"e":URL_SAFE_NO_PAD.encode(key.e().to_bytes_be())}]}).to_string();
    let verifier = VerifierStore::new(
        Verifier::from_trusted_snapshot(
            "https://synthetic.invalid",
            "finance-api",
            &jwks,
            now(),
            600,
        )
        .unwrap(),
    );
    let signer = EncodingKey::from_rsa_der(key.to_pkcs1_der().unwrap().as_bytes());
    let mint = |sub: &str| {
        let mut hdr = Header::new(Algorithm::RS256);
        hdr.typ = Some("at+jwt".into());
        hdr.kid = Some("http-test".into());
        let time = now();
        encode(
            &hdr,
            &json!({"iss":"https://synthetic.invalid","sub":sub,"aud":"finance-api","iat":time,"exp":time+300,"role":"ORG_ADMIN"}),
            &signer,
        )
        .unwrap()
    };
    let requester = mint("requester");
    let policy_admin = mint("policy-admin");
    let approver = mint("form-checker");
    let auditor = mint("auditor");
    let scope = CompanyScope {
        tenant: TenantId::try_from(id(1)).unwrap(),
        company: CompanyId::try_from(id(10)).unwrap(),
    };
    let app = router(OrganizationState::synthetic(
        runtime, lookup, verifier, scope, [73; 32],
    ));

    // 1. Dossier + 2 documents for upload-grant, finalize, OCR, classify, extract, supersede
    let (ds_status, ds) = create_dossier(&app, &requester, "wave1-dossier-1", "PAYMENT").await;
    assert_eq!(ds_status, StatusCode::CREATED);
    let dossier_id = ds["id"].as_str().unwrap().to_string();

    let (other_status, other_ds) =
        create_dossier(&app, &requester, "wave1-dossier-2", "ADVANCE").await;
    assert_eq!(other_status, StatusCode::CREATED);
    let other_dossier_id = other_ds["id"].as_str().unwrap().to_string();

    let docs_uri = format!("/v1/dossiers/{dossier_id}/documents");
    let (d1_status, d1, d1_ver) =
        create_document(&app, &requester, &docs_uri, &dossier_id, "w1-doc-1", 0, 512).await;
    assert_eq!(d1_status, StatusCode::CREATED);
    let doc1_id = d1["document_id"].as_str().unwrap().to_string();
    let v1 = d1_ver.unwrap();

    let (d2_status, d2, d2_ver) =
        create_document(&app, &requester, &docs_uri, &dossier_id, "w1-doc-2", 1, 512).await;
    assert_eq!(d2_status, StatusCode::CREATED);
    let doc2_id = d2["document_id"].as_str().unwrap().to_string();
    let mut ver = d2_ver.unwrap();

    let grant_uri = format!("/v1/dossiers/{dossier_id}/documents/{doc1_id}/upload-grants");
    let (g1_st, g1, g1_ver) =
        post_contract(&app, &requester, &grant_uri, "w1-grant-1", Some(&ver), None).await;
    assert_eq!(g1_st, StatusCode::CREATED);
    assert_eq!(g1["document_id"], doc1_id);
    assert_eq!(g1["generation"], 1);
    assert_eq!(g1["checksum_sha256_base64"].as_str().unwrap().len(), 44);
    assert_ne!(g1_ver.as_deref(), Some(ver.as_str()));

    let (g1_replay_st, g1_replay, _) =
        post_contract(&app, &requester, &grant_uri, "w1-grant-1", Some(&ver), None).await;
    assert_eq!(g1_replay_st, StatusCode::CREATED);
    assert_eq!(g1_replay["upload_url"], g1["upload_url"]);

    let v_after_g1 = g1_ver.unwrap();
    let (g2_st, g2, g2_ver) = post_contract(
        &app,
        &requester,
        &grant_uri,
        "w1-grant-2",
        Some(&v_after_g1),
        None,
    )
    .await;
    assert_eq!(g2_st, StatusCode::CREATED);
    assert_eq!(g2["generation"], 2);
    ver = g2_ver.unwrap();

    // Superseded grant replay must fail with 409 UPLOAD_GRANT_INACTIVE
    let (sup_replay_st, sup_replay, _) = post_contract(
        &app,
        &requester,
        &grant_uri,
        "w1-grant-1",
        Some(&v1),
        None,
    )
    .await;
    assert_eq!(sup_replay_st, StatusCode::CONFLICT);
    assert_eq!(sup_replay["code"], "UPLOAD_GRANT_INACTIVE");

    let finalize_uri = format!("/v1/dossiers/{dossier_id}/documents/{doc1_id}/finalize");
    let (fin_st, fin_job, fin_ver) = post_contract(
        &app,
        &requester,
        &finalize_uri,
        "w1-finalize-1",
        Some(&ver),
        Some(json!({"sha256":"a".repeat(64),"generation":2})),
    )
    .await;
    assert_eq!(fin_st, StatusCode::ACCEPTED);
    assert_eq!(fin_job["state"], "QUEUED");
    let fin_job_id = fin_job["id"].as_str().unwrap();
    let (job_st, job_row) =
        request(&app, &format!("/v1/jobs/{fin_job_id}"), Some(&requester)).await;
    assert_eq!(job_st, StatusCode::OK);
    assert_eq!(job_row["id"], fin_job_id);
    ver = fin_ver.unwrap();

    let (ocr_st, ocr_job, ocr_ver) = post_contract(
        &app,
        &requester,
        &format!("/v1/dossiers/{dossier_id}/documents/{doc1_id}/ocr"),
        "w1-ocr-1",
        Some(&ver),
        Some(json!({"reason":"Re-run synthetic OCR"})),
    )
    .await;
    assert_eq!(ocr_st, StatusCode::ACCEPTED);
    assert_eq!(ocr_job["state"], "QUEUED");
    ver = ocr_ver.unwrap();

    let (cls_st, cls_job, cls_ver) = post_contract(
        &app,
        &requester,
        &format!("/v1/dossiers/{dossier_id}/documents/{doc1_id}/classify"),
        "w1-cls-1",
        Some(&ver),
        Some(json!({"kind":"CONTRACT"})),
    )
    .await;
    assert_eq!(cls_st, StatusCode::ACCEPTED);
    assert_eq!(cls_job["state"], "QUEUED");
    ver = cls_ver.unwrap();

    let (ext_st, ext_job, ext_ver) = post_contract(
        &app,
        &requester,
        &format!("/v1/dossiers/{dossier_id}/documents/{doc1_id}/extract"),
        "w1-ext-1",
        Some(&ver),
        Some(json!({"reason":"Re-run structured extraction"})),
    )
    .await;
    assert_eq!(ext_st, StatusCode::ACCEPTED);
    assert_eq!(ext_job["state"], "QUEUED");
    ver = ext_ver.unwrap();

    let (sup_st, sup_dossier, sup_ver) = post_contract(
        &app,
        &requester,
        &format!("/v1/dossiers/{dossier_id}/documents/{doc1_id}/supersede"),
        "w1-sup-1",
        Some(&ver),
        Some(json!({"replacement_id":doc2_id,"reason":"Replaced by corrected invoice"})),
    )
    .await;
    assert_eq!(sup_st, StatusCode::OK);
    assert_eq!(sup_dossier["id"], dossier_id);
    ver = sup_ver.unwrap();

    // 2. Ready export download + signed content retrieval
    let ready_dossier = "00000000-0000-0000-0000-000000001800";
    let (exp_list_st, exp_list) = request(
        &app,
        &format!("/v1/dossiers/{ready_dossier}/exports?limit=10"),
        Some(&requester),
    )
    .await;
    assert_eq!(exp_list_st, StatusCode::OK);
    let pkg_id = exp_list["items"][0]["id"].as_str().unwrap();
    let (dl_st, dl) = request(
        &app,
        &format!("/v1/dossiers/{ready_dossier}/exports/{pkg_id}/download"),
        Some(&requester),
    )
    .await;
    assert_eq!(dl_st, StatusCode::OK);
    let dl_url = dl["url"].as_str().unwrap();
    let dl_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(dl_url)
                .header(header::AUTHORIZATION, format!("Bearer {requester}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(dl_resp.status(), StatusCode::OK);
    assert_eq!(
        dl_resp
            .headers()
            .get("x-content-sha256")
            .unwrap()
            .to_str()
            .unwrap(),
        dl["sha256"].as_str().unwrap()
    );
    assert_eq!(
        request(
            &app,
            &format!("/v1/dossiers/{ready_dossier}/exports/{pkg_id}/content?grant=tampered"),
            Some(&requester),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    // 3. Template preview
    let template_id = "00000000-0000-0000-0000-000000001804";
    let (_, tpl_cur) = request(
        &app,
        &format!("/v1/templates/{template_id}"),
        Some(&policy_admin),
    )
    .await;
    let tpl_v = format!(
        "\"v1:template:{template_id}:{}\"",
        tpl_cur["version"].as_i64().unwrap()
    );
    let (prev_st, prev_job, prev_hdr) = post_contract(
        &app,
        &policy_admin,
        &format!("/v1/templates/{template_id}/preview"),
        "w1-tpl-preview-1",
        Some(&tpl_v),
        Some(json!({"fixture_id":"00000000-0000-0000-0000-000000001a02"})),
    )
    .await;
    assert_eq!(prev_st, StatusCode::ACCEPTED);
    assert_eq!(prev_job["state"], "QUEUED");
    assert_eq!(prev_hdr.as_deref(), Some(tpl_v.as_str()));

    // 4. Reference creation + incomplete guard + SoD verification
    let evidence_id = "00000000-0000-0000-0000-000000002333";
    let (partial_st, partial_job, _) = post_contract(
        &app,
        &requester,
        "/v1/references",
        "w1-ref-partial",
        None,
        Some(json!({
            "document_id":"00000000-0000-0000-0000-000000001a02",
            "kind":"CONTRACT",
            "as_of":"2026-01-15T00:00:00Z",
            "coverage":"PARTIAL"
        })),
    )
    .await;
    assert_eq!(partial_st, StatusCode::ACCEPTED);
    let partial_ref = partial_job["id"].as_str().unwrap();
    let (partial_v_st, _, _) = post_contract(
        &app,
        &approver,
        &format!("/v1/references/{partial_ref}/verify"),
        "w1-ref-verify-partial",
        Some(&format!("\"v1:reference:{partial_ref}:0\"")),
        Some(json!({"reason":"Attempt verify partial","evidence_ids":[evidence_id]})),
    )
    .await;
    assert_eq!(partial_v_st, StatusCode::CONFLICT);

    let (pay_ref_st, pay_ref_job, _) = post_contract(
        &app,
        &requester,
        "/v1/references",
        "w1-ref-pay-1",
        None,
        Some(json!({
            "document_id":"00000000-0000-0000-0000-000000001a02",
            "kind":"PAYMENT",
            "as_of":"2026-01-15T00:00:00Z",
            "coverage":"COMPLETE"
        })),
    )
    .await;
    assert_eq!(pay_ref_st, StatusCode::ACCEPTED);
    let pay_ref_id = pay_ref_job["id"].as_str().unwrap();
    let ref_v0 = format!("\"v1:reference:{pay_ref_id}:0\"");
    assert_eq!(
        post_contract(
            &app,
            &requester,
            &format!("/v1/references/{pay_ref_id}/verify"),
            "w1-ref-sod",
            Some(&ref_v0),
            Some(json!({"reason":"Self verify rejected","evidence_ids":[evidence_id]})),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (verify_st, verified, verify_ver) = post_contract(
        &app,
        &approver,
        &format!("/v1/references/{pay_ref_id}/verify"),
        "w1-ref-verify-pay",
        Some(&ref_v0),
        Some(json!({"reason":"Verified synthetic payment reference","evidence_ids":[evidence_id]})),
    )
    .await;
    assert_eq!(verify_st, StatusCode::OK);
    assert_eq!(verified["state"], "VERIFIED");
    assert_eq!(verified["kind"], "PAYMENT");
    assert_eq!(
        verify_ver.as_deref(),
        Some(format!("\"v1:reference:{pay_ref_id}:1\"").as_str())
    );
    assert_eq!(
        request(&app, "/v1/references?limit=100", Some(&auditor)).await.0,
        StatusCode::FORBIDDEN
    );

    // 5. Payment observation & fail-closed default on unobserved dossier
    let (obs_st, obs, obs_ver) = post_contract(
        &app,
        &approver,
        &format!("/v1/dossiers/{dossier_id}/payment-observations"),
        "w1-pay-obs-1",
        Some(&ver),
        Some(json!({
            "reference_id": pay_ref_id,
            "evidence_ids": [evidence_id],
            "reason": "Observed treasury disbursement statement"
        })),
    )
    .await;
    assert_eq!(obs_st, StatusCode::OK);
    assert_eq!(obs["state"], "PAID");
    assert_eq!(obs["confirmed_paid"]["amount"], "1000000");
    assert_eq!(obs["confirmed_paid"]["currency"], "VND");
    assert!(obs_ver.is_some());

    let (paid_st, paid) = request(
        &app,
        &format!("/v1/dossiers/{dossier_id}/payment-status"),
        Some(&requester),
    )
    .await;
    assert_eq!(paid_st, StatusCode::OK);
    assert_eq!(paid["state"], "PAID");
    assert_eq!(paid["confirmed_paid"]["amount"], "1000000");

    let (unk_st, unk) = request(
        &app,
        &format!("/v1/dossiers/{other_dossier_id}/payment-status"),
        Some(&requester),
    )
    .await;
    assert_eq!(unk_st, StatusCode::OK);
    assert_eq!(unk["state"], "UNKNOWN");
}
