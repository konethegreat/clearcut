// Written by Kone Tshivhinda & Claude
//! Starts the job runner next to the HTTP server.
//!
//! What it reads from the settings (`.env` or the environment):
//!
//! - `CLEARCUT_WORKER_URL` and `CLEARCUT_WORKER_TOKEN`: the document worker. Without them no
//!   runner starts and documents are stored but not read.
//! - **The Anthropic key is per tenant** (ADR-0019): each job asks [`TenantKeys`] for its tenant's
//!   key, and the server's own `ANTHROPIC_API_KEY` is used only in `shared` mode (your own
//!   computer). A tenant with no usable key has its documents analysed and then **wait**, without
//!   using up attempts, and start within a minute of a key being set.
//!
//! **One job at a time.** The spend gate that lets documents share a monthly budget safely
//! while running together is not built (P2-12), so the runner does one document at a time.

use crate::local_env::LocalEnv;
use clearcut_anthropic::provider::AnthropicProvider;
use clearcut_anthropic::AnthropicConfig;
use clearcut_docproc::HttpWorker;
use clearcut_jobs::{Runner, RunnerConfig};
use clearcut_keys::{Resolved, TenantKeys};
use clearcut_objects::LocalDisk;
use clearcut_pipeline::{
    NoProvider, NoticeFuture, PipelineConfig, PipelineHandler, ProviderFuture, ProviderSource, ReadSettings,
};
use clearcut_store::{Store, TenantId};
use std::sync::Arc;
use tokio::sync::watch::Receiver;
use tokio::task::JoinHandle;

/// Gives each job the provider for its tenant's own key.
struct TenantProviders {
    keys: TenantKeys,
    config: AnthropicConfig,
}

impl ProviderSource<AnthropicProvider> for TenantProviders {
    fn provider<'a>(&'a self, tenant: TenantId) -> ProviderFuture<'a, AnthropicProvider> {
        Box::pin(async move {
            match self.keys.resolve(tenant).await {
                Ok(Resolved::Key(key, source)) => match AnthropicProvider::new(&self.config, &key) {
                    Ok(provider) => {
                        tracing::info!(source = source.as_str(), key = %key.fingerprint(), "reading with an Anthropic key");
                        Ok(Arc::new(provider))
                    }
                    Err(error) => {
                        tracing::error!(%error, "an Anthropic client could not be made; the job will wait");
                        Err(NoProvider::NoKey)
                    }
                },
                Ok(Resolved::NoKey) => Err(NoProvider::NoKey),
                Ok(Resolved::Rejected) => Err(NoProvider::Rejected),
                Err(error) => {
                    tracing::error!(%error, "the tenant's key could not be looked up; the job will wait");
                    Err(NoProvider::NoKey)
                }
            }
        })
    }

    fn key_rejected<'a>(&'a self, tenant: TenantId) -> NoticeFuture<'a> {
        Box::pin(async move {
            if let Err(error) = self.keys.mark_rejected(tenant).await {
                tracing::error!(%error, "a rejected tenant key could not be marked");
            }
        })
    }
}

/// Starts the runner if a worker is configured. `Err` is a message for the person starting the
/// server (never a secret).
pub fn start(
    env: &LocalEnv,
    store: &Store,
    objects: LocalDisk,
    keys: TenantKeys,
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
    let anthropic = AnthropicConfig::from_env(|name| env.os(name)).map_err(|e| format!("{e}"))?;
    let handler = PipelineHandler::new(store.clone(), objects, worker, PipelineConfig::default())
        .with_provider_source(
            Arc::new(TenantProviders {
                keys,
                config: anthropic,
            }),
            ReadSettings::standard(),
        );
    Ok(Some(spawn(Runner::new(store.clone(), handler, config), stopped)))
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
