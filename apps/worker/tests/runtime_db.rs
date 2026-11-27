use std::{
    fs,
    os::unix::fs::PermissionsExt,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
#[ignore = "requires disposable PostgreSQL after API export request"]
fn worker_binary_processes_export_with_fenced_adapter_receipt() {
    let database = std::env::var("FINANCE_JOB_TEST_DATABASE_URL").unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("finance-worker-{nonce}"));
    fs::create_dir_all(&root).unwrap();
    let adapter = root.join("adapter.sh");
    let script = r#"#!/bin/sh
set -eu
job=''
fence=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --job-id) job="$2"; shift 2 ;;
    --fence) fence="$2"; shift 2 ;;
    *) shift 2 ;;
  esac
done
test -n "$job"
test -n "$fence"
d='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
printf '{"schema":"finance.export.receipt.v1","job_id":"%s","fence":%s,"artifacts":[{"name":"request.docx","expected_sha256":"%s","observed_sha256":"%s","bytes":100},{"name":"checklist.xlsx","expected_sha256":"%s","observed_sha256":"%s","bytes":120},{"name":"request.pdf","expected_sha256":"%s","observed_sha256":"%s","bytes":140}]}' "$job" "$fence" "$d" "$d" "$d" "$d" "$d" "$d"
"#;
    fs::write(&adapter, script).unwrap();
    let mut permissions = fs::metadata(&adapter).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&adapter, permissions).unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_finance-worker"))
        .arg("--once")
        .env("FINANCE_WORKER_DATABASE_URL", database)
        .env(
            "FINANCE_WORKER_TENANT_ID",
            "00000000-0000-0000-0000-000000000001",
        )
        .env(
            "FINANCE_WORKER_COMPANY_ID",
            "00000000-0000-0000-0000-00000000000a",
        )
        .env(
            "FINANCE_WORKER_ID",
            "00000000-0000-0000-0000-000000006001",
        )
        .env("FINANCE_WORKER_LEASE_SECONDS", "30")
        .env("FINANCE_EXPORT_ADAPTER_TIMEOUT_SECONDS", "10")
        .env("FINANCE_EXPORT_ADAPTER", &adapter)
        .status()
        .unwrap();
    let _ = fs::remove_dir_all(&root);
    assert!(status.success(), "runtime worker failed export job");
}
