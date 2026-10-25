// Written by Kone Tshivhinda & Claude
//! The ladder as data (ADR-0018 section 2).

use super::ThinkingEffort;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// The most a provider will take as the largest answer (the API's own limit).
const PROVIDER_MAX_OUTPUT_TOKENS: u32 = 64_000;
/// The longest model id the provider takes.
const MAX_MODEL_ID_LEN: usize = 128;

// The page picture's limits are the worker's (`/v1/render`): dpi 100 to 300, long side 500 to
// 2 576 pixels (what the models we use take).
const MIN_DPI: u16 = 100;
const MAX_DPI: u16 = 300;
const MIN_EDGE: u16 = 500;
const MAX_EDGE: u16 = 2_576;

/// How a page is drawn for the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Picture {
    pub dpi: u16,
    /// The longest side, in pixels.
    pub max_edge: u16,
}

/// One rung of the ladder: a model, how hard it thinks, and what it is given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rung {
    /// The pinned model id, such as `claude-haiku-5-5`.
    pub model: String,
    /// Fixed within a rung: a changed effort breaks the prompt cache.
    pub effort: ThinkingEffort,
    /// The largest answer, which includes the model's thinking. It is also the worst-case bill
    /// of one call, so it is chosen so a call fits the document's budget.
    pub max_output_tokens: u32,
    pub picture: Picture,
    pub enabled: bool,
}

/// The ladder and the caps on calls. Built with [`RouterPolicy::new`] (which checks it), or
/// [`RouterPolicy::default_ladder`]. **Read back from text, it goes through the same checks**
/// (`try_from`), so a configuration file cannot hold a policy that `new` would refuse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedPolicy")]
pub struct RouterPolicy {
    rungs: Vec<Rung>,
    /// What a cut-off answer is retried with (the adapter's non-streaming limit by default).
    answer_ceiling: u32,
    max_calls_per_page: u32,
    document_calls_per_page: u32,
    document_calls_extra: u32,
}

/// A policy as it is written, before it is checked.
#[derive(Deserialize)]
struct UncheckedPolicy {
    rungs: Vec<Rung>,
    answer_ceiling: u32,
    max_calls_per_page: u32,
    document_calls_per_page: u32,
    document_calls_extra: u32,
}

impl TryFrom<UncheckedPolicy> for RouterPolicy {
    type Error = Error;

    fn try_from(raw: UncheckedPolicy) -> Result<Self> {
        RouterPolicy::new(
            raw.rungs,
            raw.answer_ceiling,
            raw.max_calls_per_page,
            raw.document_calls_per_page,
            raw.document_calls_extra,
        )
    }
}

fn invalid(why: &'static str) -> Error {
    Error::InvalidRouterPolicy(why)
}

/// The answer ceiling must be a size the provider takes.
pub(super) fn check_ceiling(answer_ceiling: u32) -> Result<()> {
    if answer_ceiling == 0 || answer_ceiling > PROVIDER_MAX_OUTPUT_TOKENS {
        return Err(invalid("the answer ceiling is outside 1 to 64000"));
    }
    Ok(())
}

/// What a single rung must satisfy, whoever built it.
pub(super) fn check_rung(rung: &Rung, answer_ceiling: u32) -> Result<()> {
    if !plain_model_id(&rung.model) {
        return Err(invalid("a model id is not a plain identifier"));
    }
    if rung.max_output_tokens == 0 || rung.max_output_tokens > answer_ceiling {
        return Err(invalid("a rung's answer cap is outside 1 to the ceiling"));
    }
    let Picture { dpi, max_edge } = rung.picture;
    if !(MIN_DPI..=MAX_DPI).contains(&dpi) || !(MIN_EDGE..=MAX_EDGE).contains(&max_edge) {
        return Err(invalid("a rung's picture is outside what the worker draws"));
    }
    Ok(())
}

fn plain_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_MODEL_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

impl RouterPolicy {
    /// Checks and keeps a policy. `answer_ceiling` is what a cut-off answer is retried with.
    /// A page may make `max_calls_per_page` calls; a document of `n` pages may make
    /// `document_calls_per_page x n + document_calls_extra`.
    pub fn new(
        rungs: Vec<Rung>,
        answer_ceiling: u32,
        max_calls_per_page: u32,
        document_calls_per_page: u32,
        document_calls_extra: u32,
    ) -> Result<Self> {
        if rungs.is_empty() {
            return Err(invalid("a ladder needs at least one rung"));
        }
        if !rungs.iter().any(|r| r.enabled) {
            return Err(invalid("at least one rung must be on"));
        }
        check_ceiling(answer_ceiling)?;
        if max_calls_per_page == 0 || document_calls_per_page == 0 {
            return Err(invalid("a call cap must be above zero"));
        }
        for (i, rung) in rungs.iter().enumerate() {
            check_rung(rung, answer_ceiling)?;
            if rungs[..i].iter().any(|earlier| earlier.model == rung.model) {
                return Err(invalid("a model appears twice in the ladder"));
            }
        }
        Ok(RouterPolicy {
            rungs,
            answer_ceiling,
            max_calls_per_page,
            document_calls_per_page,
            document_calls_extra,
        })
    }

    /// The ladder of ADR-0018 section 2: Haiku, Sonnet and Opus on, Fable off (it keeps data for
    /// 30 days and an account may refuse it). Starting values for the gold set to correct.
    pub fn default_ladder() -> Self {
        let rung = |model: &str, effort, max_output_tokens, dpi, max_edge, enabled| Rung {
            model: model.to_string(),
            effort,
            max_output_tokens,
            picture: Picture { dpi, max_edge },
            enabled,
        };
        RouterPolicy {
            rungs: vec![
                rung(
                    "claude-haiku-5-5",
                    ThinkingEffort::Medium,
                    16_000,
                    150,
                    2_000,
                    true,
                ),
                rung(
                    "claude-sonnet-5-5",
                    ThinkingEffort::Medium,
                    12_000,
                    150,
                    2_000,
                    true,
                ),
                rung("claude-opus-5-5", ThinkingEffort::High, 8_000, 200, 2_576, true),
                rung("claude-fable-5-1", ThinkingEffort::High, 4_000, 200, 2_576, false),
            ],
            answer_ceiling: 16_000,
            max_calls_per_page: 5,
            document_calls_per_page: 3,
            document_calls_extra: 10,
        }
    }

    /// Every rung, in order, on or off. A rung is named by its position here.
    pub fn rungs(&self) -> &[Rung] {
        &self.rungs
    }

    /// The rungs that are on, in order.
    pub fn effective(&self) -> Vec<&Rung> {
        self.rungs.iter().filter(|r| r.enabled).collect()
    }

    pub fn answer_ceiling(&self) -> u32 {
        self.answer_ceiling
    }

    pub fn max_calls_per_page(&self) -> u32 {
        self.max_calls_per_page
    }

    /// The most calls a document of `pages` pages may make, however the pages go.
    pub fn document_call_cap(&self, pages: u32) -> u32 {
        self.document_calls_per_page
            .saturating_mul(pages)
            .saturating_add(self.document_calls_extra)
    }

    /// The same policy with one model switched on or off. The result is checked again, so the
    /// last rung that is on cannot be switched off.
    pub fn with_enabled(&self, model: &str, enabled: bool) -> Result<Self> {
        let mut rungs = self.rungs.clone();
        let rung = rungs
            .iter_mut()
            .find(|r| r.model == model)
            .ok_or_else(|| invalid("no such model in the ladder"))?;
        rung.enabled = enabled;
        RouterPolicy::new(
            rungs,
            self.answer_ceiling,
            self.max_calls_per_page,
            self.document_calls_per_page,
            self.document_calls_extra,
        )
    }

    /// The same policy without the models a key cannot use (from the Models API, or an answer
    /// that named the model). Not checked: a key may be able to use none, and the router then
    /// says so (`no_rung_available`) instead of failing to start.
    pub fn without_models(&self, models: &[&str]) -> Self {
        let mut narrowed = self.clone();
        narrowed.rungs.retain(|r| !models.contains(&r.model.as_str()));
        narrowed
    }
}
