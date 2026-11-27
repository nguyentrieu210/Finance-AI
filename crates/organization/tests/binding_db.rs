use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use finance_audit::read_page_authenticated;
use finance_core::prototype::{CreateUnit, Error, Receipt, UnitKind, create_unit_authenticated};
use finance_identity::{IdentityError, Verifier, VerifierStore};
use finance_organization::{UpdateUnit, list_units_authenticated, update_unit_authenticated};
use finance_shared::{
    contracts::{Principal, Version},
    http_contract::IdempotencyKey,
    ids::*,
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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
fn scope(c: u128) -> CompanyScope {
    CompanyScope {
        tenant: TenantId::try_from(id(1)).unwrap(),
        company: CompanyId::try_from(id(c)).unwrap(),
    }
}
fn command(n: u128) -> CommandId {
    CommandId::try_from(id(n)).unwrap()
}
fn corr(n: u128) -> CorrelationId {
    CorrelationId::try_from(id(n + 10000)).unwrap()
}
fn idem(s: &str) -> IdempotencyKey {
    IdempotencyKey::parse(s).unwrap()
}
#[tokio::test]
#[ignore = "requires disposable synthetic PostgreSQL"]
async fn authenticated_commands_real_database() {
    let runtime = PgPoolOptions::new()
        .max_connections(3)
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE finance_runtime").execute(c).await?;
                Ok(())
            })
        })
        .connect(&std::env::var("FINANCE_PROTOTYPE_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let lookup = PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE finance_identity_lookup")
                    .execute(c)
                    .await?;
                Ok(())
            })
        })
        .connect(&std::env::var("FINANCE_IDENTITY_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&std::env::var("FINANCE_AUDIT_TEST_ADMIN_URL").unwrap())
        .await
        .unwrap();
    let key = RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).unwrap();
    let jwks=json!({"keys":[{"kty":"RSA","kid":"test-key","alg":"RS256","n":URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()),"e":URL_SAFE_NO_PAD.encode(key.e().to_bytes_be())}]}).to_string();
    let store = VerifierStore::new(
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
    let mint = |sub: &str, ttl: u64| {
        let mut header = Header::new(Algorithm::RS256);
        header.typ = Some("at+jwt".into());
        header.kid = Some("test-key".into());
        let t = now();
        encode(&header,&json!({"iss":"https://synthetic.invalid","sub":sub,"aud":"finance-api","iat":t,"exp":t+ttl,"role":"FINANCE_APPROVER","actor_id":id(9999)}),&signer).unwrap()
    };
    let token = mint("org-admin", 180);
    let proof = store
        .authenticate_command(&lookup, &token, scope(10), command(9501), corr(9501), None)
        .await
        .unwrap();
    assert_eq!(
        proof.context().actor,
        Principal::User(ActorId::try_from(id(1000)).unwrap())
    );
    assert!(matches!(
        store
            .authenticate_command(&lookup, &token, scope(99), command(9502), corr(9502), None)
            .await,
        Err(IdentityError::Forbidden)
    ));
    let input = CreateUnit {
        id: id(9503),
        parent: None,
        code: "AUTH-BOUND",
        name: "Verified synthetic command",
        kind: UnitKind::Department,
    };
    assert_eq!(
        create_unit_authenticated(&runtime, &proof, &idem("bound"), &input)
            .await
            .unwrap(),
        Receipt::Created(id(9503))
    );
    assert_eq!(
        create_unit_authenticated(&runtime, &proof, &idem("bound"), &input)
            .await
            .unwrap(),
        Receipt::Replayed(id(9503))
    );
    let update = store
        .authenticate_command(
            &lookup,
            &token,
            scope(10),
            command(9504),
            corr(9504),
            Some(Version::try_from(0).unwrap()),
        )
        .await
        .unwrap();
    let result = update_unit_authenticated(
        &runtime,
        &update,
        &idem("bound-update"),
        &UpdateUnit {
            id: id(9503),
            parent: None,
            name: "Updated",
            active: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(i64::from(result.version), 1);
    assert!(
        list_units_authenticated(&runtime, &proof, Uuid::nil(), 100)
            .await
            .unwrap()
            .iter()
            .any(|u| u.id == id(9503) && u.version == 1)
    );
    let auditor = store
        .authenticate_command(
            &lookup,
            &mint("auditor", 180),
            scope(10),
            command(9505),
            corr(9505),
            None,
        )
        .await
        .unwrap();
    assert!(
        read_page_authenticated(&runtime, &auditor, 0, 100)
            .await
            .unwrap()
            .iter()
            .any(|e| e.resource == id(9503))
    );
    assert!(matches!(
        create_unit_authenticated(&runtime, &auditor, &idem("denied"), &input).await,
        Err(Error::Forbidden)
    ));
    // Identity mapping change after authentication invalidates the proof even if role still exists.
    sqlx::query(
        "UPDATE finance.identity_subject SET subject='changed-admin' WHERE tenant_id=$1 AND id=$2",
    )
    .bind(id(1))
    .bind(id(1000))
    .execute(&admin)
    .await
    .unwrap();
    assert!(matches!(
        create_unit_authenticated(&runtime, &proof, &idem("bound"), &input).await,
        Err(Error::Forbidden)
    ));
    sqlx::query(
        "UPDATE finance.identity_subject SET subject='org-admin' WHERE tenant_id=$1 AND id=$2",
    )
    .bind(id(1))
    .bind(id(1000))
    .execute(&admin)
    .await
    .unwrap();
    // Hold the company tree lock beyond token expiry; the actual mutation must fail closed.
    let mut blocker = admin.begin().await.unwrap();
    sqlx::query("SELECT finance.lock_org_tree($1,$2)")
        .bind(id(1))
        .bind(id(10))
        .execute(&mut *blocker)
        .await
        .unwrap();
    let short = store
        .authenticate_command(
            &lookup,
            &mint("org-admin", 3),
            scope(10),
            command(9506),
            corr(9506),
            None,
        )
        .await
        .unwrap();
    let expired_input = CreateUnit {
        id: id(9507),
        parent: None,
        code: "EXPIRED-BOUND",
        name: "Must not persist",
        kind: UnitKind::CostCenter,
    };
    let short_key = idem("short-expiry");
    let pending = create_unit_authenticated(&runtime, &short, &short_key, &expired_input);
    tokio::pin!(pending);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), &mut pending)
            .await
            .is_err()
    );
    tokio::time::sleep(Duration::from_secs(4)).await;
    blocker.commit().await.unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), &mut pending)
            .await
            .unwrap(),
        Err(Error::Forbidden)
    ));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM finance.organization_unit WHERE id=$1")
            .bind(id(9507))
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(count, 0);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM finance.audit_event WHERE command_id=$1")
            .bind(id(9506))
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(count, 0);
    runtime.close().await;
    lookup.close().await;
    admin.close().await;
}
