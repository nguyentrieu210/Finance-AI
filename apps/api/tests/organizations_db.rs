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
