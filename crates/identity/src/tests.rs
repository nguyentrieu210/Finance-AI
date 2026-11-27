use super::*;
use jsonwebtoken::{EncodingKey, Header as JwtHeader, encode};
use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};
use serde_json::{Value, json};
use std::sync::OnceLock;
fn key(k: &str) -> &'static RsaPrivateKey {
    static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
    static ROTATED: OnceLock<RsaPrivateKey> = OnceLock::new();
    let slot = if k == "key2" { &ROTATED } else { &KEY };
    slot.get_or_init(|| RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).unwrap())
}
fn jwk(k: &str) -> Value {
    json!({"kty":"RSA","kid":k,"alg":"RS256","use":"sig","n":URL_SAFE_NO_PAD.encode(key(k).n().to_bytes_be()),"e":URL_SAFE_NO_PAD.encode(key(k).e().to_bytes_be())})
}
fn snapshot(k: &str) -> String {
    json!({"keys":[jwk(k)]}).to_string()
}
fn token(k: &str, claims: &Value, typ: &str) -> String {
    let mut header = JwtHeader::new(Algorithm::RS256);
    header.kid = Some(k.into());
    header.typ = Some(typ.into());
    encode(
        &header,
        claims,
        &EncodingKey::from_rsa_der(key(k).to_pkcs1_der().unwrap().as_bytes()),
    )
    .unwrap()
}
fn claims() -> Value {
    json!({"iss":"https://synthetic.invalid","sub":"org-admin","aud":"finance-api","iat":100,"exp":200})
}
fn verifier(k: &str) -> Verifier {
    Verifier::from_trusted_snapshot(
        "https://synthetic.invalid",
        "finance-api",
        &snapshot(k),
        100,
        120,
    )
    .unwrap()
}
#[test]
fn verified_signature_and_claims() {
    let v = verifier("key1");
    let c = claims();
    assert!(v.verify_at(&token("key1", &c, "at+jwt"), 150).is_ok());
    for (field, bad) in [
        ("iss", json!("https://other.invalid")),
        ("aud", json!("other")),
        ("sub", json!("")),
        ("exp", json!(150)),
        ("iat", json!(151)),
        ("nbf", json!(151)),
        ("exp", json!(4000)),
    ] {
        let mut c = claims();
        c[field] = bad;
        assert!(
            matches!(
                v.verify_at(&token("key1", &c, "at+jwt"), 150),
                Err(IdentityError::InvalidToken)
            ),
            "accepted {field}"
        );
    }
    let mut c = claims();
    c.as_object_mut().unwrap().remove("iat");
    assert!(v.verify_at(&token("key1", &c, "at+jwt"), 150).is_err());
    assert!(v.verify_at(&token("key1", &claims(), "JWT"), 150).is_err());
}
#[test]
fn rotation_expiry_signature_and_algorithm_fail_closed() {
    let old = verifier("key1");
    let new = verifier("key2");
    let t = token("key1", &claims(), "at+jwt");
    assert!(old.verify_at(&t, 150).is_ok());
    assert!(new.verify_at(&t, 150).is_err());
    assert!(
        new.verify_at(&token("key2", &claims(), "at+jwt"), 150)
            .is_ok()
    );
    assert!(matches!(
        old.verify_at(&t, 220),
        Err(IdentityError::ExpiredKeys)
    ));
    let parts: Vec<_> = t.split('.').collect();
    let mut sig = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
    sig[0] ^= 1;
    assert!(
        old.verify_at(
            &format!("{}.{}.{}", parts[0], parts[1], URL_SAFE_NO_PAD.encode(sig)),
            150
        )
        .is_err()
    );
    for hdr in [
        json!({"alg":"none","typ":"at+jwt","kid":"key1"}),
        json!({"alg":"HS256","typ":"at+jwt","kid":"key1"}),
        json!({"alg":"RS256","typ":"at+jwt","kid":"key1","jku":"https://evil.invalid"}),
        json!({"alg":"RS256","typ":"at+jwt","kid":"key1","crit":["x"]}),
    ] {
        assert!(
            old.verify_at(
                &format!(
                    "{}.{}.{}",
                    URL_SAFE_NO_PAD.encode(hdr.to_string()),
                    parts[1],
                    parts[2]
                ),
                150
            )
            .is_err()
        );
    }
}
#[test]
fn reject_duplicate_weak_or_untrusted_key_metadata() {
    let mut weak = jwk("k");
    weak["n"] = json!(URL_SAFE_NO_PAD.encode([255u8; 128]));
    for keys in [
        json!([jwk("k"), jwk("k")]),
        json!([weak]),
        json!([]),
        json!([{"kty":"oct","kid":"k","k":"AAAA"}]),
    ] {
        assert!(matches!(
            Verifier::from_trusted_snapshot(
                "https://synthetic.invalid",
                "finance-api",
                &json!({"keys":keys}).to_string(),
                100,
                120
            ),
            Err(IdentityError::InvalidKeys)
        ));
    }
    assert!(
        Verifier::from_trusted_snapshot(
            "http://synthetic.invalid",
            "finance-api",
            &snapshot("k"),
            100,
            120
        )
        .is_err()
    );
}
#[tokio::test]
#[ignore = "requires isolated synthetic PostgreSQL"]
async fn identity_real_database() {
    use finance_shared::ids::{CompanyId, TenantId};
    use sqlx::postgres::PgPoolOptions;
    let pool = PgPoolOptions::new()
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
    let id = Uuid::from_u128;
    let scope = |t, c| CompanyScope {
        tenant: TenantId::try_from(id(t)).unwrap(),
        company: CompanyId::try_from(id(c)).unwrap(),
    };
    let n = now().unwrap();
    let v = Verifier::from_trusted_snapshot(
        "https://synthetic.invalid",
        "finance-api",
        &snapshot("k"),
        n,
        300,
    )
    .unwrap();
    let mut claims = claims();
    claims["iat"] = json!(n);
    claims["exp"] = json!(n + 180);
    claims["sub"] = json!("requester");
    let t = token("k", &claims, "at+jwt");
    assert_eq!(
        v.authenticate(&pool, &t, scope(1, 10)).await,
        Ok(Principal::User(ActorId::try_from(id(1001)).unwrap()))
    );
    assert_eq!(
        v.authenticate(&pool, &t, scope(1, 11)).await,
        Err(IdentityError::Forbidden)
    );
    assert_eq!(
        v.authenticate(&pool, &t, scope(2, 20)).await,
        Err(IdentityError::Forbidden)
    );
    for sub in ["expired", "unknown"] {
        claims["sub"] = json!(sub);
        assert_eq!(
            v.authenticate(&pool, &token("k", &claims, "at+jwt"), scope(1, 10))
                .await,
            Err(IdentityError::Forbidden)
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM finance.identity_subject")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(
        sqlx::query("UPDATE finance.membership SET active=true")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(sqlx::query("INSERT INTO finance.identity_subject(tenant_id,id,issuer,subject) VALUES(gen_random_uuid(),gen_random_uuid(),'a','b')").execute(&pool).await.is_err());
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&std::env::var("FINANCE_AUDIT_TEST_ADMIN_URL").unwrap())
        .await
        .unwrap();
    sqlx::query("UPDATE finance.membership SET active=false WHERE tenant_id=$1 AND company_id=$2 AND actor_id=$3").bind(id(1)).bind(id(10)).bind(id(1001)).execute(&admin).await.unwrap();
    assert_eq!(
        v.authenticate(&pool, &t, scope(1, 10)).await,
        Err(IdentityError::Forbidden)
    );
    pool.close().await;
    admin.close().await;
}

#[test]
fn atomic_rotation_rejects_old_and_rollback_snapshot() {
    let store = VerifierStore::new(verifier("key1"));
    let old_token = token("key1", &claims(), "at+jwt");
    assert!(store.verify_at(&old_token, 150).is_ok());
    let newer = Verifier::from_trusted_snapshot(
        "https://synthetic.invalid",
        "finance-api",
        &snapshot("key2"),
        151,
        120,
    )
    .unwrap();
    assert_ne!(key("key1").n(), key("key2").n());
    assert_eq!(store.replace(newer), Ok(()));
    assert!(store.verify_at(&old_token, 152).is_err());
    assert!(
        store
            .verify_at(&token("key2", &claims(), "at+jwt"), 152)
            .is_ok()
    );
    assert_eq!(
        store.replace(verifier("key1")),
        Err(IdentityError::InvalidConfig)
    );
    assert!(matches!(
        store.verify_at(&old_token, 271),
        Err(IdentityError::ExpiredKeys)
    ));
}
