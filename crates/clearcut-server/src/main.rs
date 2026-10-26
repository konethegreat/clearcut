// Written by Kone Tshivhinda & Claude
mod local_env;
mod runner;

use clearcut_intake::Intake;
use clearcut_objects::LocalDisk;
use clearcut_server::{app, AppState};
use clearcut_store::Store;
use local_env::LocalEnv;
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

/// The largest upload accepted unless `CLEARCUT_MAX_UPLOAD_BYTES` says otherwise: 64 MiB.
const DEFAULT_MAX_UPLOAD_BYTES: u64 = 64 * 1024 * 1024;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    // An Anthropic key is never accepted on the command line (other users of the computer can
    // read a program's arguments).
    if clearcut_anthropic::reject_key_in_arguments(std::env::args().skip(1)).is_err() {
        eprintln!(
            "error: never put an API key on the command line; use ANTHROPIC_API_KEY_FILE or the .env file"
        );
        return ExitCode::from(2);
    }
    let env = LocalEnv::load();

    // The service connects as the restricted `clearcut_app` role (ADR-0007),
    // NOT as the owner. Migrations and tenants are the operator's job
    // (`clearcut-admin`), so nothing here changes the schema.
    let Some(url) = env.get("DATABASE_URL") else {
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
    let Some(storage_dir) = env.get("CLEARCUT_STORAGE_DIR") else {
        eprintln!("error: CLEARCUT_STORAGE_DIR must be set to the directory where uploaded files are kept");
        return ExitCode::from(2);
    };
    let max_upload_bytes = match env.get("CLEARCUT_MAX_UPLOAD_BYTES") {
        None => DEFAULT_MAX_UPLOAD_BYTES,
        Some(value) => match value.trim().parse::<u64>() {
            Ok(bytes) if bytes > 0 => bytes,
            _ => {
                eprintln!(
                    "error: CLEARCUT_MAX_UPLOAD_BYTES must be a whole number of bytes greater than zero"
                );
                return ExitCode::from(2);
            }
        },
    };
    let objects = LocalDisk::new(storage_dir);
    let intake = Intake::new(store.clone(), objects.clone(), max_upload_bytes);

    // Stop both the server and the job runner on Ctrl+C.
    let (stop, stopped) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = stop.send(true);
    });

    // The job runner reads documents in the background (the document worker, and a model if a
    // key is set). Without a worker URL documents are stored but not read.
    let runner = match runner::start(&env, &store, objects, stopped.clone()) {
        Ok(handle) => handle,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };

    let addr = env
        .get("CLEARCUT_BIND")
        .unwrap_or_else(|| "127.0.0.1:8080".into());
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("error: could not listen on {addr}: {error}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(%addr, "clearcut-server listening");
    let mut server_stop = stopped;
    let served = axum::serve(
        listener,
        app(AppState::new(store.clone())
            .with_ingest(intake)
            .with_jobs(store.clone())
            .with_reviews(store.clone())
            .with_results(store)),
    )
    .with_graceful_shutdown(async move {
        let _ = server_stop.wait_for(|stopped| *stopped).await;
    })
    .await;
    if let Some(runner) = runner {
        let _ = runner.await;
    }
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
