//! CI-only real generated PNG -> Tesseract TSV -> PostgreSQL evidence and candidates.
use finance_document_intelligence::{
    Kind,
    normalize::{Format, Outcome},
};
use finance_extraction::{
    Field, State,
    rows::{OwnedValue, extract},
};
use finance_ocr::tsv::parse_page;
use finance_shared::ids::{CompanyId, CompanyScope, TenantId};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[tokio::test]
#[ignore = "disposable PostgreSQL and actual synthetic OCR result only"]
async fn real_candidate_persistence() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&std::env::var("FINANCE_OCR_DB_URL").unwrap())
        .await
        .unwrap();
    let worker = PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE finance_job_worker")
                    .execute(c)
                    .await?;
                Ok(())
            })
        })
        .connect(&std::env::var("FINANCE_OCR_WORKER_URL").unwrap())
        .await
        .unwrap();
    let t = id(1);
    let c = id(10);
    let a = id(1001);
    let d = id(8001);
    let r = id(8002);
    let doc = id(8003);
    let job = id(8004);
    let w = id(8005);
    let source = std::fs::read(std::env::var("FINANCE_OCR_ROWS_IMAGE").unwrap()).unwrap();
    let input = std::fs::read_to_string(std::env::var("FINANCE_OCR_ROWS_TSV").unwrap()).unwrap();
    assert!(source.starts_with(b"\x89PNG\r\n\x1a\n"));
    let digest = Sha256::digest(&source).to_vec();
    let model = std::fs::read(std::env::var("FINANCE_OCR_TRAINEDDATA").unwrap()).unwrap();
    let model = Sha256::digest(&model).to_vec();
    let config = Sha256::digest(b"tesseract/vie/psm6/1800x480/rows.v1").to_vec();
    sqlx::query("INSERT INTO finance.dossier(tenant_id,company_id,id,owner_id,code,kind) VALUES($1,$2,$3,$4,'SYN-OCR','PAYMENT')").bind(t).bind(c).bind(d).bind(a).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO finance.dossier_revision(tenant_id,company_id,dossier_id,id,owner_id,number) VALUES($1,$2,$3,$4,$5,0)").bind(t).bind(c).bind(d).bind(r).bind(a).execute(&pool).await.unwrap();
    sqlx::query(
        "SELECT finance.prototype_reserve_upload($1,$2,$3,$4,$5,$6,'rows.png',$7,$8,'image/png')",
    )
    .bind(t)
    .bind(c)
    .bind(doc)
    .bind(d)
    .bind(r)
    .bind(a)
    .bind(source.len() as i64)
    .bind(&digest)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("SELECT finance.prototype_issue_upload_grant($1,$2,$3,$4,$5,300)")
        .bind(t)
        .bind(c)
        .bind(doc)
        .bind(r)
        .bind(a)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("SELECT finance.prototype_finalize_synthetic_upload($1,$2,$3,$4,$5,1,$6)")
        .bind(t)
        .bind(c)
        .bind(doc)
        .bind(r)
        .bind(a)
        .bind(&source)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO finance.outbox(tenant_id,company_id,id,command_id,resource_id,event_type) VALUES($1,$2,$3,$3,$4,'SYNTHETIC_OCR_REQUESTED')").bind(t).bind(c).bind(job).bind(doc).execute(&pool).await.unwrap();
    let scope = CompanyScope {
        tenant: TenantId::try_from(t).unwrap(),
        company: CompanyId::try_from(c).unwrap(),
    };
    assert_eq!(
        finance_core::jobs::dispatch(&worker, scope, 10)
            .await
            .unwrap(),
        1
    );
    let lease = finance_core::jobs::claim(&worker, scope, w, 120)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lease.job_id, job);
    let page = parse_page(&input, 1, 1, 1800, 480).unwrap();
    let blocks=Value::Array(page.blocks().iter().map(|b|json!({"rect":b.rect().coordinates(),"text":b.text(),"confidence":b.confidence()})).collect());
    let results = extract(
        Kind::Invoice,
        std::slice::from_ref(&page),
        Format::Vietnamese,
    )
    .unwrap();
    let mut fields = Vec::new();
    for field in results {
        let path = match field.field {
            Field::InvoiceNumber => "invoice.number",
            Field::DocumentDate => "invoice.date",
            Field::Supplier => "invoice.supplier",
            Field::AmountBeforeTax => "invoice.net",
            Field::TaxAmount => "invoice.tax",
            Field::TotalAmount => "invoice.total",
            Field::ContractNumber => "invoice.contract_number",
            Field::RequestedAmount => "invoice.requested",
            Field::Currency => "invoice.currency",
        };
        let state = match field.state {
            State::Unknown => "UNKNOWN",
            State::Invalid => "INVALID",
            State::Ambiguous => "AMBIGUOUS",
            State::Candidate => "CANDIDATE",
        };
        let mut value = None::<String>;
        let mut indexes = Vec::<usize>::new();
        let mut source_page = None::<u32>;
        if field.state == State::Candidate {
            assert_eq!(field.observations.len(), 1);
            let obs = &field.observations[0];
            if let Outcome::Candidate(v) = &obs.outcome {
                value = Some(match v {
                    OwnedValue::Amount(a) => a.decimal().to_string(),
                    OwnedValue::Date(d) => d.to_string(),
                    OwnedValue::Identifier(s) | OwnedValue::Text(s) => s.clone(),
                });
                indexes = obs.block_indexes.clone();
                source_page = Some(obs.page);
            }
        }
        fields.push(
            json!({"path":path,"state":state,"value":value,"page":source_page,"indexes":indexes}),
        );
    }
    let fields = Value::Array(fields);
    let submit = |f, source_digest: &Vec<u8>| {
        sqlx::query_scalar::<_,Uuid>("SELECT finance.prototype_record_synthetic_ocr($1,$2,$3,$4,$5,$6,$7,1,$8,$9,$10,$11,$12)")
            .bind(t).bind(c).bind(job).bind(f).bind(w).bind(doc).bind(r).bind(source_digest.clone()).bind(model.clone()).bind(config.clone()).bind(blocks.clone()).bind(fields.clone())
    };
    assert!(
        submit(lease.fence + 1, &digest)
            .fetch_one(&pool)
            .await
            .is_err()
    );
    assert!(
        submit(lease.fence, &vec![0; 32])
            .fetch_one(&pool)
            .await
            .is_err()
    );
    assert_eq!(
        submit(lease.fence, &digest).fetch_one(&pool).await.unwrap(),
        job
    );
    assert!(submit(lease.fence, &digest).fetch_one(&pool).await.is_err());
    let total:(String,Option<String>)=sqlx::query_as("SELECT state,normalized_value FROM finance.extracted_field WHERE run_id=$1 AND field_path='invoice.total'").bind(job).fetch_one(&pool).await.unwrap();
    assert_eq!(total, ("CANDIDATE".into(), Some("125000".into())));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM finance.evidence WHERE ocr_run_id=$1")
            .bind(job)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, page.blocks().len() as i64);
    assert!(
        sqlx::query("UPDATE finance.evidence SET excerpt='changed' WHERE ocr_run_id=$1")
            .bind(job)
            .execute(&pool)
            .await
            .is_err()
    );
    let state:(String,String)=sqlx::query_as("SELECT j.state,b.scan_state FROM finance.job j JOIN finance.ocr_run o ON (o.tenant_id,o.company_id,o.job_id)=(j.tenant_id,j.company_id,j.id) JOIN finance.document_blob b ON (b.tenant_id,b.company_id,b.id)=(o.tenant_id,o.company_id,o.document_id) WHERE j.id=$1").bind(job).fetch_one(&pool).await.unwrap();
    assert_eq!(state, ("SUCCEEDED".into(), "QUARANTINED".into()));
    pool.close().await;
    worker.close().await;
}
