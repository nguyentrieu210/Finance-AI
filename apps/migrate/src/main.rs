#![forbid(unsafe_code)]
use sqlx::{migrate::Migrator, postgres::PgPoolOptions};
use std::{process::ExitCode, time::Duration};

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

#[tokio::main]
async fn main() -> ExitCode {
    // Migrations are a separate operator command; API and worker never run them automatically.
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--apply"] {
        eprintln!("Usage: finance-migrate --apply (requires FINANCE_MIGRATION_DATABASE_URL)");
        return ExitCode::from(64);
    }
    let Ok(url) = std::env::var("FINANCE_MIGRATION_DATABASE_URL") else {
        eprintln!("FINANCE_MIGRATION_DATABASE_URL is required");
        return ExitCode::from(78);
    };
    if url.trim().is_empty() {
        eprintln!("FINANCE_MIGRATION_DATABASE_URL must not be empty");
        return ExitCode::from(78);
    }
    let pool = match PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&url)
        .await
    {
        Ok(pool) => pool,
        Err(_) => {
            // Never print connection strings or database errors containing secrets.
            eprintln!("Migration database connection failed");
            return ExitCode::FAILURE;
        }
    };
    let result = MIGRATOR.run(&pool).await;
    pool.close().await;
    match result {
        Ok(()) => {
            println!("SQLx migration ledger validated and migrations applied");
            ExitCode::SUCCESS
        }
        Err(_) => {
            eprintln!("SQLx migration failed: inspect authorized database/migration diagnostics");
            ExitCode::FAILURE
        }
    }
}
