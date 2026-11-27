use axum::{
    Router,
    body::{Body,to_bytes},
    http::{Request,StatusCode,header},
};
use base64::{Engine,engine::general_purpose::URL_SAFE_NO_PAD};
use finance_api::organizations::{OrganizationState,router};
use finance_identity::{Verifier,VerifierStore};
use finance_shared::ids::{CompanyId,CompanyScope,TenantId};
use jsonwebtoken::{Algorithm,EncodingKey,Header,encode};
use rsa::{RsaPrivateKey,pkcs1::EncodeRsaPrivateKey,traits::PublicKeyParts};
use serde_json::{Value,json};
use sqlx::postgres::PgPoolOptions;
use std::time::{SystemTime,UNIX_EPOCH};
use tower::ServiceExt;
use uuid::Uuid;

fn id(n:u128)->Uuid{Uuid::from_u128(n)}
fn now()->u64{SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()}

async fn get(app:&Router,uri:&str,token:&str)->(StatusCode,Value){
    let response=app.clone().oneshot(
        Request::builder().uri(uri).header(header::AUTHORIZATION,format!("Bearer {token}"))
        .body(Body::empty()).unwrap()
    ).await.unwrap();
    let status=response.status();
    let bytes=to_bytes(response.into_body(),1024*1024).await.unwrap();
    (status,serde_json::from_slice(&bytes).unwrap())
}

async fn decide(
    app:&Router,token:&str,dossier:&str,key:&str,version:u64,field:&Value,value:&str
)->(StatusCode,Value,Option<String>){
    let response=app.clone().oneshot(
        Request::builder().method("POST")
        .uri(format!("/v1/dossiers/{dossier}/fields/decisions"))
        .header(header::AUTHORIZATION,format!("Bearer {token}"))
        .header("idempotency-key",key)
        .header("if-finance-version",format!("\"v1:dossier:{dossier}:{version}\""))
        .header(header::CONTENT_TYPE,"application/json")
        .body(Body::from(json!({
            "field_id":field["id"],
            "value":value,
            "reason":"Independent check against actual OCR evidence",
            "evidence_ids":field["evidence_ids"],
        }).to_string())).unwrap()
    ).await.unwrap();
    let status=response.status();
    let receipt=response.headers().get("finance-version").and_then(|v|v.to_str().ok()).map(str::to_owned);
    let bytes=to_bytes(response.into_body(),1024*1024).await.unwrap();
    (status,serde_json::from_slice(&bytes).unwrap_or_else(|_|json!({})),receipt)
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
#[ignore="requires disposable PostgreSQL containing actual synthetic OCR persistence"]
async fn actual_ocr_fields_http_review(){
    let runtime=PgPoolOptions::new().max_connections(3).after_connect(|c,_|Box::pin(async move{
        sqlx::query("SET ROLE finance_runtime").execute(c).await?;Ok(())
    })).connect(&std::env::var("FINANCE_OCR_RUNTIME_URL").unwrap()).await.unwrap();
    let lookup=PgPoolOptions::new().max_connections(2).after_connect(|c,_|Box::pin(async move{
        sqlx::query("SET ROLE finance_identity_lookup").execute(c).await?;Ok(())
    })).connect(&std::env::var("FINANCE_OCR_IDENTITY_URL").unwrap()).await.unwrap();

    let private=RsaPrivateKey::new(&mut rand::rngs::OsRng,2048).unwrap();
    let jwks=json!({"keys":[{"kty":"RSA","kid":"field-http","alg":"RS256","n":URL_SAFE_NO_PAD.encode(private.n().to_bytes_be()),"e":URL_SAFE_NO_PAD.encode(private.e().to_bytes_be())}]}).to_string();
    let verifier=VerifierStore::new(Verifier::from_trusted_snapshot(
        "https://synthetic.invalid","finance-api",&jwks,now(),600
    ).unwrap());
    let signer=EncodingKey::from_rsa_der(private.to_pkcs1_der().unwrap().as_bytes());
    let mint=|sub:&str|{
        let mut h=Header::new(Algorithm::RS256);h.typ=Some("at+jwt".into());h.kid=Some("field-http".into());
        let time=now();
        encode(&h,&json!({"iss":"https://synthetic.invalid","sub":sub,"aud":"finance-api","iat":time,"exp":time+300}),&signer).unwrap()
    };
    let requester=mint("requester");
    let accountant=mint("field-accountant");
    let auditor=mint("auditor");
    let app=router(OrganizationState::synthetic(
        runtime.clone(),lookup.clone(),verifier,
        CompanyScope{tenant:TenantId::try_from(id(1)).unwrap(),company:CompanyId::try_from(id(10)).unwrap()},
        [89;32],
    ));
    let dossier="00000000-0000-0000-0000-000000001f41";
    let (status,page)=get(&app,&format!("/v1/dossiers/{dossier}/fields?limit=100"),&requester).await;
    assert_eq!(status,StatusCode::OK);
    let fields=page["items"].as_array().unwrap();
    assert_eq!(fields.len(),6);
    let total=fields.iter().find(|f|f["path"]=="invoice.total").unwrap().clone();
    assert_eq!(total["state"],"CANDIDATE");
    assert_eq!(total["normalized_value"],"125000");
    assert!(!total["evidence_ids"].as_array().unwrap().is_empty());
    assert_eq!(get(&app,&format!("/v1/dossiers/{dossier}/fields"),&auditor).await.0,StatusCode::FORBIDDEN);

    assert_eq!(decide(&app,&requester,dossier,"field-owner-denied",0,&total,"125000").await.0,StatusCode::FORBIDDEN);
    let (decision_status,verified,receipt)=decide(&app,&accountant,dossier,"field-review-one",0,&total,"125000").await;
    assert_eq!(decision_status,StatusCode::OK);
    assert_eq!(verified["state"],"VERIFIED");
    assert_eq!(verified["normalized_value"],"125000");
    assert_eq!(receipt.as_deref(),Some("\"v1:dossier:00000000-0000-0000-0000-000000001f41:1\""));

    let replay=decide(&app,&accountant,dossier,"field-review-one",0,&total,"125000").await;
    assert_eq!(replay.0,StatusCode::OK);
    assert_eq!(replay.1,verified);
    assert_eq!(replay.2,receipt);
    assert_eq!(decide(&app,&accountant,dossier,"field-review-one",0,&total,"125001").await.0,StatusCode::CONFLICT);
    assert_eq!(decide(&app,&accountant,dossier,"field-review-stale",0,&total,"125000").await.0,StatusCode::CONFLICT);

    let (_,after)=get(&app,&format!("/v1/dossiers/{dossier}/fields?limit=100"),&requester).await;
    let current=after["items"].as_array().unwrap().iter().find(|f|f["path"]=="invoice.total").unwrap();
    assert_eq!(current["state"],"VERIFIED");
    assert_eq!(current["normalized_value"],"125000");
    runtime.close().await;lookup.close().await;
}
