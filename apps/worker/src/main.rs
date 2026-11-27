#![forbid(unsafe_code)]
fn main() -> std::process::ExitCode {
    eprintln!("finance-worker is disabled: durable jobs and handlers are not implemented at G00.");
    std::process::ExitCode::from(78)
}
