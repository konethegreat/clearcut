// Written by Kone Tshivhinda & Claude
//! The log of every call the model router made or considered (ADR-0018 section 8, P2-15).
//!
//! **No content at all**: ids, names, sizes, token counts and rand. The pack's manifest is
//! kept as JSON (section names and sizes, hashes, counts). The log is **append-only**, with no
//! way to change or remove a row, and it is **kept when a document is erased** (it has no
//! foreign key to the document): the month's spending is added up from it, and what a document
//! cost is not the document's content.
//!
//! A row's `cost_nano_rand` is what the budget was charged. For a call that may have been
//! billed but reported nothing (a timeout) that is the call's worst case, not a bill.

use crate::{DocId, Error, Result, Store, TenantId};
use clearcut_core::cost::{Cost, TokenUsage};
use clearcut_core::escalation::{CallKind, Picture, ThinkingEffort};
use serde_json::Value;
use std::time::Duration;
use time::OffsetDateTime;
use uuid::Uuid;

/// The most calls one `record_model_calls` may carry.
pub const MAX_BATCH: usize = 1_000;
/// The largest manifest, as JSON text (the table's limit is 16 384 on the stored form, which is
/// larger than the text).
pub const MAX_MANIFEST_BYTES: usize = 15_000;

/// The labels the router uses (`clearcut_router::CallLabel`).
const LABELS: [&str; 10] = [
    "accepted",
    "escalated",
    "reviewed",
    "invalid_output",
    "truncated",
    "refused",
    "rejected",
    "rung_unavailable",
    "not_sent",
    "interrupted",
];

/// One call, to be logged. Everything here is a size, a name, a count or an amount.
#[derive(Debug, Clone, PartialEq)]
pub struct NewModelCall {
    pub job: Option<Uuid>,
    pub document: DocId,
    pub page: u32,
    pub attempt: u32,
    /// The rung of the ladder (a position in the policy's rungs).
    pub rung: u8,
    pub model: String,
    pub effort: ThinkingEffort,
    pub picture: Picture,
    pub kind: CallKind,
    pub prompt_version: String,
    /// The pack's manifest as JSON; `None` when no request was built.
    pub manifest: Option<Value>,
    pub usage: TokenUsage,
    /// What the budget was charged.
    pub cost: Cost,
    pub elapsed: Duration,
    pub request_id: Option<String>,
    /// One of the router's labels (`accepted`, `escalated`, ...).
    pub label: String,
    /// The reason or kind that goes with the label: lower-case words joined by underscores.
    pub label_detail: Option<String>,
}

fn is_detail(text: &str) -> bool {
    (1..=40).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

fn check_call(call: &NewModelCall) -> Result<()> {
    let invalid = Error::InvalidModelCall;
    if call.page == 0 {
        return Err(invalid("pages start at 1"));
    }
    if !(1..=50).contains(&call.attempt) {
        return Err(invalid("the attempt is outside 1 to 50"));
    }
    if call.rung > 7 {
        return Err(invalid("the rung is outside the ladder"));
    }
    if let CallKind::Escalation { from, .. } = call.kind {
        if from > 7 {
            return Err(invalid("the rung is outside the ladder"));
        }
    }
    if call.model.is_empty() || call.model.chars().count() > 100 {
        return Err(invalid("the model name is empty or too long"));
    }
    if call.prompt_version.is_empty() || call.prompt_version.chars().count() > 64 {
        return Err(invalid("the prompt version is empty or too long"));
    }
    if !(100..=300).contains(&call.picture.dpi) || !(500..=2_576).contains(&call.picture.max_edge) {
        return Err(invalid("the picture size is outside what the worker draws"));
    }
    if let Some(manifest) = &call.manifest {
        let size = serde_json::to_string(manifest).map_or(usize::MAX, |text| text.len());
        if !manifest.is_object() || size > MAX_MANIFEST_BYTES {
            return Err(invalid("a manifest is a JSON object of at most 15000 bytes"));
        }
    }
    if call.usage.cache_write_1h_tokens > call.usage.cache_write_tokens {
        return Err(invalid("one-hour cache writes are part of the cache writes"));
    }
    if call
        .request_id
        .as_ref()
        .is_some_and(|id| id.chars().count() > 100)
    {
        return Err(invalid("a request id is at most 100 characters"));
    }
    if !LABELS.contains(&call.label.as_str()) {
        return Err(invalid("the label is not one the router uses"));
    }
    if call
        .label_detail
        .as_deref()
        .is_some_and(|detail| !is_detail(detail))
    {
        return Err(invalid("a detail is lower-case words joined by underscores"));
    }
    Ok(())
}

/// A count or an amount as the database holds it: no larger than the column.
fn clamp(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// The kind of call as stored: its name, where it came from and why (escalations only).
fn kind_columns(kind: CallKind) -> (&'static str, Option<i16>, Option<&'static str>) {
    match kind {
        CallKind::First => ("first", None, None),
        CallKind::Escalation { from, reason } => {
            // Checked above to be at most 7.
            ("escalation", i16::try_from(from).ok(), Some(reason.as_str()))
        }
        CallKind::RetryTruncated => ("retry_truncated", None, None),
        CallKind::RetryInvalid => ("retry_invalid", None, None),
    }
}

impl Store {
    /// Logs calls, all or none. Nothing here is ever changed or removed afterwards.
    pub async fn record_model_calls(&self, tenant: TenantId, calls: &[NewModelCall]) -> Result<()> {
        if calls.len() > MAX_BATCH {
            return Err(Error::InvalidModelCall("too many calls at once"));
        }
        for call in calls {
            check_call(call)?;
        }
        let mut tx = self.begin(tenant).await?;
        for call in calls {
            let (kind, from, reason) = kind_columns(call.kind);
            sqlx::query(
                "insert into model_calls \
                 (tenant_id, document_id, job_id, page, attempt, rung, model, effort, picture_dpi, \
                  picture_max_edge, call_kind, escalated_from, escalation_reason, prompt_version, manifest, \
                  input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cache_write_1h_tokens, \
                  cost_nano_rand, elapsed_ms, request_id, label, label_detail) \
                 values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, \
                         $19, $20, $21, $22, $23, $24, $25)",
            )
            .bind(tenant.0)
            .bind(call.document.0)
            .bind(call.job)
            // Checked above: 1 to 50, and a page number that fits.
            .bind(i32::try_from(call.page).unwrap_or(i32::MAX))
            .bind(i32::try_from(call.attempt).unwrap_or(i32::MAX))
            .bind(i16::from(call.rung))
            .bind(&call.model)
            .bind(call.effort.as_str())
            .bind(i16::try_from(call.picture.dpi).unwrap_or(i16::MAX))
            .bind(i16::try_from(call.picture.max_edge).unwrap_or(i16::MAX))
            .bind(kind)
            .bind(from)
            .bind(reason)
            .bind(&call.prompt_version)
            .bind(&call.manifest)
            .bind(clamp(call.usage.input_tokens))
            .bind(clamp(call.usage.output_tokens))
            .bind(clamp(call.usage.cache_read_tokens))
            .bind(clamp(call.usage.cache_write_tokens))
            .bind(clamp(call.usage.cache_write_1h_tokens))
            .bind(clamp(call.cost.nano_rand()))
            .bind(i32::try_from(call.elapsed.as_millis()).unwrap_or(i32::MAX))
            .bind(&call.request_id)
            .bind(&call.label)
            .bind(&call.label_detail)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// What this tenant's calls were charged, in all, from `since` (inclusive) to now. The
    /// caller decides what a month is. A sum larger than a cost can hold is the largest cost.
    pub async fn spend_since(&self, tenant: TenantId, since: OffsetDateTime) -> Result<Cost> {
        let mut tx = self.begin(tenant).await?;
        // The sum is taken as text: it can exceed a 64-bit integer, and the answer is only
        // ever compared with a limit.
        let total: String = sqlx::query_scalar(
            "select coalesce(sum(cost_nano_rand), 0)::text from model_calls \
             where tenant_id = $1 and created_at >= $2",
        )
        .bind(tenant.0)
        .bind(since)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        let nano = total.parse::<u64>().unwrap_or(u64::MAX);
        Ok(Cost::from_nano_rand(nano))
    }
}

#[cfg(test)]
mod tests;
