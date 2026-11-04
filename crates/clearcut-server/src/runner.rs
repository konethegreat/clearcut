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
//! **How many at once.** `CLEARCUT_RUNNER_CONCURRENCY` (1 to 16, default 1) is how many documents are
//! read at the same time, but **never two of the same account**: the queue hands out at most one job per
//! tenant. That is what makes the spend gate unnecessary by construction: a document's budget is
//! rebuilt from the account's call log when it starts, so two documents of one account never race on it,
//! and different accounts spend different keys. On your own computer 1 is right; a deployment with many
//! accounts raises it.

use crate::local_env::LocalEnv;
use clearcut_anthropic::models::KeyCheck;
use clearcut_anthropic::provider::AnthropicProvider;
use clearcut_anthropic::AnthropicConfig;
use clearcut_core::escalation::RouterPolicy;
use clearcut_docproc::HttpWorker;
use clearcut_jobs::{Runner, RunnerConfig};
use clearcut_keys::{Resolved, TenantKeys};
use clearcut_objects::LocalDisk;
use clearcut_pipeline::{
    LimitsFuture, ModelLimits, NoProvider, NoticeFuture, PipelineConfig, PipelineHandler, ProviderFuture,
    ProviderSource, ReadSettings,
};
use clearcut_store::{Store, TenantId};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::watch::Receiver;
use tokio::task::JoinHandle;

/// How long a key's model list is believed. The check is free, but a document does not need to
/// ask again within an hour.
const LIMITS_TTL: Duration = Duration::from_secs(60 * 60);

struct CachedLimits {
    /// The last four characters, so a newly set key is checked again.
    fingerprint: String,
    checked: Instant,
    limits: ModelLimits,
}

/// Gives each job the provider for its tenant's own key.
struct TenantProviders {
    keys: TenantKeys,
    config: AnthropicConfig,
    limits: Mutex<HashMap<TenantId, CachedLimits>>,
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

    fn limits<'a>(&'a self, tenant: TenantId) -> LimitsFuture<'a> {
        Box::pin(async move { self.limits_for(tenant).await })
    }
}

impl TenantProviders {
    /// What the tenant's key can use, cached for an hour per key. A check that fails leaves the
    /// full ladder in place (the provider skips an unusable rung itself).
    async fn limits_for(&self, tenant: TenantId) -> Option<ModelLimits> {
        let Ok(Resolved::Key(key, _)) = self.keys.resolve(tenant).await else {
            return None;
        };
        let fingerprint = key.fingerprint();
        if let Ok(cache) = self.limits.lock() {
            if let Some(cached) = cache.get(&tenant) {
                if cached.fingerprint == fingerprint && cached.checked.elapsed() < LIMITS_TTL {
                    return Some(cached.limits.clone());
                }
            }
        }
        let check: KeyCheck = match self.keys.check(&key).await {
            Ok(check) => check,
            Err(error) => {
                tracing::warn!(%error, "the key's model list could not be read; the full ladder is used");
                return None;
            }
        };
        let limits = limits_from(&check);
        if let Ok(mut cache) = self.limits.lock() {
            cache.insert(
                tenant,
                CachedLimits {
                    fingerprint,
                    checked: Instant::now(),
                    limits: limits.clone(),
                },
            );
        }
        Some(limits)
    }
}

/// The ladder models the key cannot use, and the limits of those it can. A model that is absent
/// from the list, retired, or unable to take structured output or a picture is left out. An
/// answer that names none of the ladder's models is treated as no answer at all: it must not
/// empty the ladder for an hour.
fn limits_from(check: &KeyCheck) -> ModelLimits {
    let ladder = RouterPolicy::default_ladder();
    let mut limits = ModelLimits::default();
    for rung in ladder.rungs() {
        let usable = check
            .model(&rung.model)
            .filter(|info| check.can_use(&rung.model) && info.structured_outputs && info.image_input);
        match usable {
            Some(info) => {
                if let Some(max) = info.max_tokens {
                    limits
                        .max_output_tokens
                        .insert(rung.model.clone(), u32::try_from(max).unwrap_or(u32::MAX).max(1));
                }
                if let Some(max) = info.max_input_tokens {
                    limits.max_input_tokens.insert(rung.model.clone(), max);
                }
            }
            None => limits.unavailable.push(rung.model.clone()),
        }
    }
    if limits.unavailable.len() == ladder.rungs().len() {
        tracing::warn!("the key's model list names none of the ladder's models; the full ladder is used");
        return ModelLimits::default();
    }
    limits
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
        .with_concurrency(runner_concurrency(
            env.get("CLEARCUT_RUNNER_CONCURRENCY").as_deref(),
        )?)
        .map_err(|e| e.to_string())?
        .with_per_tenant_cap(Some(1));
    let anthropic = AnthropicConfig::from_env(|name| env.os(name)).map_err(|e| format!("{e}"))?;
    let handler = PipelineHandler::new(store.clone(), objects, worker, PipelineConfig::default())
        .with_provider_source(
            Arc::new(TenantProviders {
                keys,
                config: anthropic,
                limits: Mutex::new(HashMap::new()),
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

/// `CLEARCUT_RUNNER_CONCURRENCY`: a whole number from 1 to 16; unset means 1.
fn runner_concurrency(text: Option<&str>) -> Result<usize, String> {
    match text.map(str::trim).filter(|t| !t.is_empty()) {
        None => Ok(1),
        Some(text) => text
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=16).contains(n))
            .ok_or_else(|| "CLEARCUT_RUNNER_CONCURRENCY must be a whole number from 1 to 16".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{limits_from, runner_concurrency, ModelLimits};
    use clearcut_anthropic::models::{KeyCheck, ModelInfo};
    use clearcut_anthropic::Lifecycle;

    #[test]
    fn the_concurrency_is_one_unless_asked_and_is_bounded() {
        assert_eq!(runner_concurrency(None), Ok(1));
        assert_eq!(runner_concurrency(Some(" ")), Ok(1));
        assert_eq!(runner_concurrency(Some("4")), Ok(4));
        for bad in ["0", "17", "-1", "many", "2.5"] {
            assert!(runner_concurrency(Some(bad)).is_err(), "{bad}");
        }
    }

    fn model(id: &str, lifecycle: Lifecycle, structured: bool, image: bool) -> ModelInfo {
        ModelInfo {
            id: id.to_string(),
            display_name: id.to_string(),
            lifecycle,
            max_input_tokens: Some(200_000),
            max_tokens: Some(64_000),
            structured_outputs: structured,
            image_input: image,
            pdf_input: true,
            batch: false,
        }
    }

    #[test]
    fn a_key_check_names_the_rungs_it_cannot_use_and_the_limits_it_reports() {
        let check = KeyCheck {
            models: vec![
                model("claude-haiku-5-5", Lifecycle::Active, true, true),
                model("claude-sonnet-5-5", Lifecycle::Active, false, true),
            ],
            request_id: None,
        };
        let limits = limits_from(&check);
        assert_eq!(
            limits.unavailable,
            ["claude-sonnet-5-5", "claude-opus-5-5", "claude-fable-5-1"]
        );
        assert_eq!(limits.max_output_tokens.get("claude-haiku-5-5"), Some(&64_000));
        assert_eq!(limits.max_input_tokens.get("claude-haiku-5-5"), Some(&200_000));
    }

    #[test]
    fn an_answer_that_names_none_of_the_ladder_keeps_the_whole_ladder() {
        let empty = KeyCheck {
            models: Vec::new(),
            request_id: None,
        };
        assert_eq!(limits_from(&empty), ModelLimits::default());

        let elsewhere = KeyCheck {
            models: vec![model("claude-legacy-1", Lifecycle::Active, true, true)],
            request_id: None,
        };
        assert_eq!(limits_from(&elsewhere), ModelLimits::default());
    }
}
