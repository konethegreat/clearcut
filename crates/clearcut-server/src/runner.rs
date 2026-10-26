// Written by Kone Tshivhinda & Claude
//! Starts the job runner next to the HTTP server, for running ClearCut on one's own computer.
//!
//! What it reads from the settings (`.env` or the environment):
//!
//! - `CLEARCUT_WORKER_URL` and `CLEARCUT_WORKER_TOKEN`: the document worker. Without them no
//!   runner starts and documents are stored but not read.
//! - `ANTHROPIC_API_KEY` or `ANTHROPIC_API_KEY_FILE`: the user's own key. Without one the runner
//!   only analyses documents (how each page will be read) and asks no model.
//!
//! **One job at a time.** The spend gate that lets documents share a monthly budget safely
//! while running together is not built (P2-12), so the runner does one document at a time.

use crate::local_env::LocalEnv;
use clearcut_anthropic::provider::AnthropicProvider;
use clearcut_anthropic::{load_from, AnthropicConfig, LoadError};
use clearcut_docproc::HttpWorker;
use clearcut_jobs::{Runner, RunnerConfig};
use clearcut_objects::LocalDisk;
use clearcut_pipeline::{PipelineConfig, PipelineHandler, ReadSettings};
use clearcut_store::Store;
use std::sync::Arc;
use tokio::sync::watch::Receiver;
use tokio::task::JoinHandle;

/// Starts the runner if a worker is configured. `Err` is a message for the person starting the
/// server (never a secret).
pub fn start(
    env: &LocalEnv,
    store: &Store,
    objects: LocalDisk,
    stopped: Receiver<bool>,
) -> Result<Option<JoinHandle<()>>, String> {
    let (Some(url), Some(token)) = (env.get("CLEARCUT_WORKER_URL"), env.get("CLEARCUT_WORKER_TOKEN")) else {
        tracing::warn!("CLEARCUT_WORKER_URL and CLEARCUT_WORKER_TOKEN are not set: documents will be stored but not read");
        return Ok(None);
    };
    let worker = HttpWorker::new(&url, &token)
        .map_err(|e| format!("the document worker settings are not valid: {e}"))?;
    let worker = Arc::new(worker);
    let config = RunnerConfig::new(format!("server-{}", std::process::id()))
        .map_err(|e| e.to_string())?
        .with_concurrency(1)
        .map_err(|e| e.to_string())?;
    let handler = PipelineHandler::new(store.clone(), objects, worker, PipelineConfig::default());

    match load_from(|name| env.os(name)) {
        Err(LoadError::NotConfigured) => {
            tracing::warn!("no Anthropic key is set: documents will be analysed but no model will read them");
            Ok(Some(spawn(Runner::new(store.clone(), handler, config), stopped)))
        }
        Err(other) => Err(format!("the Anthropic key cannot be used: {other}")),
        Ok(key) => {
            let anthropic = AnthropicConfig::from_env(|name| env.os(name)).map_err(|e| format!("{e}"))?;
            let provider = AnthropicProvider::new(&anthropic, &key).map_err(|e| format!("{e}"))?;
            tracing::info!(key = %key.fingerprint(), "reading documents with the Anthropic key");
            let handler = handler.with_models(Arc::new(provider), ReadSettings::standard());
            Ok(Some(spawn(Runner::new(store.clone(), handler, config), stopped)))
        }
    }
}

fn spawn<H: clearcut_jobs::Handler>(runner: Runner<H>, mut stopped: Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        runner
            .run(async move {
                let _ = stopped.wait_for(|stopped| *stopped).await;
            })
            .await;
    })
}
