// Written by Kone Tshivhinda & Claude
mod local_env;
mod retention;
mod runner;

use clearcut_anthropic::{load_from, AnthropicConfig, LoadError, MasterKey};
use clearcut_intake::Intake;
use clearcut_keys::{AnthropicChecker, KeyMode, TenantKeys};
use clearcut_objects::LocalDisk;
use clearcut_server::{app, AppState, GuardConfig};
use clearcut_store::Store;
use local_env::LocalEnv;
use std::net::SocketAddr;
use std::num::{NonZeroU32, NonZeroUsize};
use std::process::ExitCode;
use std::time::Duration;
use tracing_subscriber::EnvFilter;

/// The largest upload accepted unless `CLEARCUT_MAX_UPLOAD_BYTES` says otherwise: 64 MiB.
const DEFAULT_MAX_UPLOAD_BYTES: u64 = 64 * 1024 * 1024;
/// The most bytes one account may store unless `CLEARCUT_TENANT_QUOTA_BYTES` says otherwise: 1 GiB.
const DEFAULT_TENANT_QUOTA_BYTES: u64 = 1024 * 1024 * 1024;
/// What each account may ask for in a minute unless `CLEARCUT_RATE_LIMIT_PER_MINUTE` says otherwise.
const DEFAULT_RATE_LIMIT_PER_MINUTE: u32 = 120;
/// What one address may ask for in a minute unless `CLEARCUT_PER_IP_PER_MINUTE` says otherwise.
const DEFAULT_PER_IP_PER_MINUTE: u32 = 300;
/// Failed credentials one address may present in a minute unless `CLEARCUT_AUTH_FAILURES_PER_MINUTE` says otherwise.
const DEFAULT_AUTH_FAILURES_PER_MINUTE: u32 = 20;
/// How long a blocked address waits unless `CLEARCUT_BLOCK_SECONDS` says otherwise.
const DEFAULT_BLOCK_SECONDS: u64 = 60;
/// Requests in flight before the server sheds load unless `CLEARCUT_MAX_CONCURRENT` says otherwise.
const DEFAULT_MAX_CONCURRENT: usize = 256;
/// The longest a request may take unless `CLEARCUT_REQUEST_TIMEOUT_SECONDS` says otherwise.
const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 120;

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
    let tenant_quota_bytes = match env.get("CLEARCUT_TENANT_QUOTA_BYTES") {
        None => DEFAULT_TENANT_QUOTA_BYTES,
        Some(value) => match value.trim().parse::<u64>() {
            Ok(bytes) if bytes > 0 => bytes,
            _ => {
                eprintln!(
                    "error: CLEARCUT_TENANT_QUOTA_BYTES must be a whole number of bytes greater than zero"
                );
                return ExitCode::from(2);
            }
        },
    };
    let rate_limit_per_minute = match env.get("CLEARCUT_RATE_LIMIT_PER_MINUTE") {
        None => std::num::NonZeroU32::new(DEFAULT_RATE_LIMIT_PER_MINUTE).unwrap(),
        Some(value) => match value
            .trim()
            .parse::<u32>()
            .ok()
            .and_then(std::num::NonZeroU32::new)
        {
            Some(limit) => limit,
            None => {
                eprintln!(
                    "error: CLEARCUT_RATE_LIMIT_PER_MINUTE must be a whole number of requests greater than zero"
                );
                return ExitCode::from(2);
            }
        },
    };
    let objects = LocalDisk::new(storage_dir);
    let limits = clearcut_intake::Limits {
        max_upload_bytes,
        tenant_quota_bytes,
    };
    let intake = Intake::new(store.clone(), objects.clone(), limits);
    // The sweep erases through its own Intake: the server's is moved into the app.
    let sweep_intake = Intake::new(store.clone(), objects.clone(), limits);

    let guard = match guard_config(&env) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };

    let retention_days = match retention::retention_days(env.get("CLEARCUT_RETENTION_DAYS").as_deref()) {
        Ok(days) => days,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };

    // Each tenant's own Anthropic key (ADR-0019): sealed with a master key from the settings, and
    // `CLEARCUT_KEY_MODE=tenant-only` makes sure the server's own key is never used.
    let tenant_keys = match tenant_keys(&env, &store) {
        Ok(keys) => keys,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };

    // Stop both the server and the job runner on Ctrl+C.
    let (stop, stopped) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = stop.send(true);
    });

    // The job runner reads documents in the background (the document worker, and a model if a
    // key is set). Without a worker URL documents are stored but not read.
    let runner = match runner::start(&env, &store, objects, tenant_keys.clone(), stopped.clone()) {
        Ok(handle) => handle,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };

    match retention_days {
        Some(days) => {
            tracing::info!(days, "retention: documents older than this many days are erased");
            retention::start(sweep_intake, store.clone(), days, stopped.clone());
        }
        None => tracing::info!("retention is off: documents are kept until their account deletes them"),
    }

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
    tracing::info!(
        per_ip_per_minute = guard.per_ip_per_minute,
        failures_per_minute = guard.failures_per_minute,
        max_concurrent = guard.max_concurrent,
        timeout_seconds = guard.timeout.as_secs(),
        trust_proxy = guard.trust_proxy,
        "the request guard is on"
    );
    let mut server_stop = stopped;
    let served = axum::serve(
        listener,
        app(AppState::new(store.clone())
            .with_ingest(intake)
            .with_jobs(store.clone())
            .with_reviews(store.clone())
            .with_keys(tenant_keys)
            .with_results(store)
            .with_rate_limit(rate_limit_per_minute)
            .with_guard(guard))
        .into_make_service_with_connect_info::<SocketAddr>(),
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

/// The tenant keys: the master key (none means tenant keys are off), the mode, and the server's
/// own key if there is one. `Err` is a message for the person starting the server (never a secret).
fn tenant_keys(env: &LocalEnv, store: &Store) -> Result<TenantKeys, String> {
    let mode = KeyMode::parse(env.get("CLEARCUT_KEY_MODE").as_deref()).map_err(str::to_string)?;
    let master = MasterKey::load_from(|name| env.os(name))
        .map_err(|e| format!("the master key cannot be used: {e}"))?;
    let server_key = match load_from(|name| env.os(name)) {
        Ok(key) => Some(key),
        Err(LoadError::NotConfigured) => None,
        Err(other) => return Err(format!("the Anthropic key cannot be used: {other}")),
    };
    let anthropic = AnthropicConfig::from_env(|name| env.os(name)).map_err(|e| e.to_string())?;
    match (mode, master.is_some(), server_key.is_some()) {
        (KeyMode::TenantOnly, false, _) => {
            return Err("CLEARCUT_KEY_MODE=tenant-only needs a master key (CLEARCUT_MASTER_KEY or CLEARCUT_MASTER_KEY_FILE): without one no tenant can bring a key".into());
        }
        (KeyMode::TenantOnly, true, true) => {
            tracing::warn!("tenant-only mode: the server's own Anthropic key is set but will never be used");
        }
        (KeyMode::Shared, false, false) => {
            tracing::warn!(
                "no Anthropic key and no master key: documents will be analysed but no model will read them"
            );
        }
        _ => {}
    }
    tracing::info!(
        mode = mode.as_str(),
        tenant_keys = master.is_some(),
        "tenant Anthropic keys"
    );
    Ok(TenantKeys::new(
        store.clone(),
        master,
        mode,
        server_key,
        AnthropicChecker(anthropic),
    ))
}

/// The limits in front of the API: per-address requests and failed credentials, the block that
/// follows too many failures, the concurrency cap, the request timeout and whether the proxy's
/// forwarded address is trusted. `Err` is a message for the person starting the server.
fn guard_config(env: &LocalEnv) -> Result<GuardConfig, String> {
    let count = |name: &str, default: u32| -> Result<NonZeroU32, String> {
        setting(
            env,
            name,
            NonZeroU32::new(default).expect("a non-zero default"),
            |value| value.parse().ok().and_then(NonZeroU32::new),
        )
        .map_err(|()| format!("{name} must be a whole number greater than zero"))
    };
    let per_ip_per_minute = count("CLEARCUT_PER_IP_PER_MINUTE", DEFAULT_PER_IP_PER_MINUTE)?;
    let failures_per_minute = count(
        "CLEARCUT_AUTH_FAILURES_PER_MINUTE",
        DEFAULT_AUTH_FAILURES_PER_MINUTE,
    )?;
    let max_concurrent = setting(
        env,
        "CLEARCUT_MAX_CONCURRENT",
        NonZeroUsize::new(DEFAULT_MAX_CONCURRENT).expect("a non-zero default"),
        |value| value.parse().ok().and_then(NonZeroUsize::new),
    )
    .map_err(|()| "CLEARCUT_MAX_CONCURRENT must be a whole number greater than zero".to_string())?;
    let block = Duration::from_secs(
        setting(env, "CLEARCUT_BLOCK_SECONDS", DEFAULT_BLOCK_SECONDS, |value| {
            value.parse::<u64>().ok().filter(|seconds| *seconds > 0)
        })
        .map_err(|()| {
            "CLEARCUT_BLOCK_SECONDS must be a whole number of seconds greater than zero".to_string()
        })?,
    );
    let timeout = Duration::from_secs(
        setting(
            env,
            "CLEARCUT_REQUEST_TIMEOUT_SECONDS",
            DEFAULT_REQUEST_TIMEOUT_SECONDS,
            |value| value.parse::<u64>().ok().filter(|seconds| *seconds > 0),
        )
        .map_err(|()| {
            "CLEARCUT_REQUEST_TIMEOUT_SECONDS must be a whole number of seconds greater than zero".to_string()
        })?,
    );
    let trust_proxy = match env.get("CLEARCUT_TRUST_PROXY") {
        None => false,
        Some(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => return Err("CLEARCUT_TRUST_PROXY must be a yes or a no (1 or 0)".into()),
        },
    };
    Ok(GuardConfig {
        per_ip_per_minute,
        failures_per_minute,
        block,
        max_concurrent,
        timeout,
        trust_proxy,
    })
}

/// A setting that is absent (blank counts as absent) or parsed by `parse`. `Err(())` means the
/// value was there but unusable.
fn setting<T: Copy>(
    env: &LocalEnv,
    name: &str,
    default: T,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<T, ()> {
    match env.get(name) {
        None => Ok(default),
        Some(value) => parse(value.trim()).ok_or(()),
    }
}
