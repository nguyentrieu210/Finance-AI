#![forbid(unsafe_code)]
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            // Only static error categories; never print a DSN, token or provider error.
            eprintln!("{message}");
            ExitCode::from(78)
        }
    }
}

async fn run() -> Result<(), &'static str> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let bind = std::env::var("FINANCE_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let (app, address, mode) = match args.as_slice() {
        [] => {
            let address = bind.parse().map_err(|_| "invalid foundation bind")?;
            (finance_api::router(), address, "foundation")
        }
        [arg] if arg == "--synthetic" => {
            let address = finance_api::synthetic::bind_address(&bind)
                .map_err(|_| "synthetic mode requires loopback")?;
            let runtime = std::env::var("FINANCE_SYNTHETIC_RUNTIME_DATABASE_URL")
                .map_err(|_| "synthetic runtime configuration missing")?;
            let lookup = std::env::var("FINANCE_SYNTHETIC_LOOKUP_DATABASE_URL")
                .map_err(|_| "synthetic identity configuration missing")?;
            let jwks = std::env::var("FINANCE_SYNTHETIC_JWKS")
                .map_err(|_| "synthetic public key snapshot missing")?;
            let app = finance_api::synthetic::build(&runtime, &lookup, &jwks)
                .await
                .map_err(|_| "synthetic dependencies rejected")?;
            (app, address, "synthetic-only")
        }
        _ => return Err("unsupported API mode"),
    };
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|_| "API bind failed")?;
    eprintln!(
        "finance-api {mode} listening on {}",
        listener
            .local_addr()
            .map_err(|_| "API address unavailable")?
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|_| "API serve failed")?;
    Ok(())
}
