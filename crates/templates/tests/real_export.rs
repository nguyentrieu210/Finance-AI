use base64::{Engine, engine::general_purpose::STANDARD};
use finance_shared::upload_contract::Sha256Digest;
use finance_templates::{Artifact, verified_manifest};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: String,
    template_sha256: String,
    template_version: u64,
    artifacts: Vec<Rendered>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rendered {
    name: String,
    bytes: u64,
    sha256: String,
    base64: String,
}

#[test]
#[ignore = "requires actual isolated office artifact generation"]
fn real_office_export() {
    let mut json=Vec::new();
    File::open(std::env::var("FINANCE_EXPORT_JSON").unwrap()).unwrap().take(4*1024*1024+1).read_to_end(&mut json).unwrap();
    assert!(json.len()<=4*1024*1024);
    let data:Envelope=serde_json::from_slice(&json).unwrap();
    assert_eq!(data.schema,"finance.export.synthetic.v1");
    assert_eq!(data.template_version,1);
    assert!(Sha256Digest::parse(&data.template_sha256).is_ok());
    assert_eq!(data.artifacts.len(),3);
    let mut receipts=Vec::new();
    for (item,name) in data.artifacts.iter().zip(["request.docx","checklist.xlsx","request.pdf"]) {
        assert_eq!(item.name,name);
        assert!(item.base64.len()<=1_400_000);
        let bytes=STANDARD.decode(&item.base64).unwrap();
        assert_eq!(bytes.len() as u64,item.bytes);
        assert!(bytes.len()<=1024*1024);
        let expected=Sha256Digest::parse(&item.sha256).unwrap();
        let observed=Sha256Digest::parse(&format!("{:x}",Sha256::digest(&bytes))).unwrap();
        if name.ends_with(".pdf"){assert!(bytes.starts_with(b"%PDF-"));}
        else{assert!(bytes.starts_with(b"PK\x03\x04"));}
        receipts.push(Artifact{name:&item.name,expected,observed:Some(observed),bytes:item.bytes});
    }
    assert!(verified_manifest(&receipts));
    receipts[0].observed=None;
    assert!(!verified_manifest(&receipts));
}
