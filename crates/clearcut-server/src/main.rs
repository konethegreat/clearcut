// Written by Kone Tshivhinda & Claude
use clearcut_intake::Intake;
use clearcut_objects::LocalDisk;
use clearcut_server::{app, AppState};
use clearcut_store::Store;
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

/// The largest upload accepted unless `CLEARCUT_MAX_UPLOAD_BYTES` says otherwise: 64 MiB.
const DEFAULT_MAX_UPLOAD_BYTES: u64 = 64 * 1024 * 1024;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // The service connects as the restricted `clearcut_app` role (ADR-0007),
    // NOT as the owner. Migrations and tenants are the operator's job
    // (`clearcut-admin`), so nothing here changes the schema.
    let Ok(url) = std::env::var("DATABASE_URL") else {
        eprintln!(
            "error: DATABASE_URL must be set to the SERVICE database connection (the clearcut_app role)"
        );
        return ExitCode::from(2);
    };
    let pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
    {
        Ok(pool) => pool,
        // The URL is deliberately not printed: it may contain a password.
        Err(error) => {
            eprintln!("error: could not connect to the database: {error}");
            return ExitCode::FAILURE;
        }
    };
    let store = Store::new(pool);

    // Running as the owner or a superuser would silently switch tenant
    // isolation off, so refuse to start rather than serve requests that way.
    if let Err(error) = store.verify_service_role().await {
        eprintln!("error: {error}");
        eprintln!("hint: DATABASE_URL for the service must be the clearcut_app role, not the owner's");
        return ExitCode::FAILURE;
    }

    // Where uploaded files are kept. Required rather than defaulted: silently
    // writing invoices to whatever directory the server happened to start in
    // would be a surprise nobody wants.
    let Some(storage_dir) = std::env::var_os("CLEARCUT_STORAGE_DIR").filter(|dir| !dir.is_empty()) else {
        eprintln!("error: CLEARCUT_STORAGE_DIR must be set to the directory where uploaded files are kept");
        return ExitCode::from(2);
    };
    let max_upload_bytes = match std::env::var("CLEARCUT_MAX_UPLOAD_BYTES") {
        Err(_) => DEFAULT_MAX_UPLOAD_BYTES,
        Ok(value) => match value.trim().parse::<u64>() {
            Ok(bytes) if bytes > 0 => bytes,
            _ => {
                eprintln!(
                    "error: CLEARCUT_MAX_UPLOAD_BYTES must be a whole number of bytes greater than zero"
                );
                return ExitCode::from(2);
            }
        },
    };
    let intake = Intake::new(store.clone(), LocalDisk::new(storage_dir), max_upload_bytes);

    let addr = std::env::var("CLEARCUT_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("error: could not listen on {addr}: {error}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(%addr, "clearcut-server listening");
    let served = axum::serve(listener, app(AppState::new(store).with_ingest(intake)))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
