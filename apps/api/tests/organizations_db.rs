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
    assert_eq!(
        create_dossier(&app, &requester, "advance-one", "ADVANCE")
            .await
            .0,
        StatusCode::CREATED
    );
    assert_eq!(
        create_dossier(&app, &requester, "settlement-one", "SETTLEMENT")
            .await
            .0,
        StatusCode::CREATED
    );
    let (status, page) = request(&app, "/v1/dossiers", Some(&requester)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["items"].as_array().unwrap().len(), 3);
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
    let (_, documents) = request(&app, &doc_uri, Some(&requester)).await;
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
    assert!(response.contains("\"items\"") && response.contains("\"DRAFT\""));
    drop(child);
    let denied = process.env("FINANCE_BIND", "0.0.0.0:0").status().unwrap();
    assert_eq!(denied.code(), Some(78));
}
