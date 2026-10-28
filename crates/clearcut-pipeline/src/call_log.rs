// Written by Kone Tshivhinda & Claude
//! What the router recorded about a call, as a row of the call log (`model_calls`).
//!
//! A [`CallRecord`] holds no content and neither does the row: sizes, names, tokens and rand.
//! This only turns the router's labels into the log's fixed names.

use clearcut_core::jobs::FailureKind;
use clearcut_router::{CallLabel, CallRecord};
use clearcut_store::model_calls::NewModelCall;
use clearcut_store::DocId;
use uuid::Uuid;

/// The log's label and detail for a call's end.
fn label_columns(label: CallLabel) -> (&'static str, Option<&'static str>) {
    match label {
        CallLabel::Accepted => ("accepted", None),
        CallLabel::Escalated(reason) => ("escalated", Some(reason.as_str())),
        CallLabel::Reviewed(reason) => ("reviewed", Some(reason.as_str())),
        CallLabel::InvalidOutput => ("invalid_output", None),
        CallLabel::Truncated => ("truncated", None),
        CallLabel::Refused => ("refused", None),
        CallLabel::Rejected => ("rejected", None),
        CallLabel::RungUnavailable => ("rung_unavailable", None),
        CallLabel::NotSent(reason) => ("not_sent", Some(reason.as_str())),
        CallLabel::Interrupted(kind) => ("interrupted", Some(failure_name(kind))),
    }
}

fn failure_name(kind: FailureKind) -> &'static str {
    match kind {
        FailureKind::Transient => "transient",
        FailureKind::ProviderUnavailable => "provider_unavailable",
        FailureKind::NeedsReview => "needs_review",
    }
}

/// One call of `page`, as the log stores it.
pub fn to_new_model_call(job: Option<Uuid>, document: DocId, page: u32, record: &CallRecord) -> NewModelCall {
    let (label, detail) = label_columns(record.label);
    NewModelCall {
        job,
        document,
        page,
        attempt: record.attempt,
        // The ladder has four rungs; a number that does not fit is refused by the store.
        rung: u8::try_from(record.rung).unwrap_or(u8::MAX),
        model: record.model.clone(),
        effort: record.effort,
        picture: record.picture,
        kind: record.kind,
        prompt_version: record.prompt_version.clone(),
        manifest: record
            .manifest
            .as_ref()
            .and_then(|manifest| serde_json::to_value(manifest).ok()),
        usage: record.usage,
        cost: record.cost,
        elapsed: record.elapsed,
        request_id: record.request_id.clone(),
        label: label.to_string(),
        label_detail: detail.map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clearcut_core::cost::{Cost, TokenUsage};
    use clearcut_core::escalation::{CallKind, EscalationReason, Picture, ReviewReason, ThinkingEffort};
    use std::time::Duration;

    fn record(label: CallLabel) -> CallRecord {
        CallRecord {
            attempt: 3,
            rung: 1,
            model: "claude-sonnet-5-5".to_string(),
            effort: ThinkingEffort::High,
            picture: Picture {
                dpi: 200,
                max_edge: 2576,
            },
            kind: CallKind::Escalation {
                from: 0,
                reason: EscalationReason::TotalsMismatch,
            },
            prompt_version: "reading-v3/prompt-v2".to_string(),
            manifest: None,
            usage: TokenUsage {
                input_tokens: 6_000,
                output_tokens: 1_000,
                cache_read_tokens: 10_000,
                cache_write_tokens: 2_000,
                cache_write_1h_tokens: 500,
            },
            cost: Cost::from_nano_rand(19_800_000),
            elapsed: Duration::from_millis(1_234),
            request_id: Some("req_1".to_string()),
            label,
        }
    }

    #[test]
    fn every_field_of_the_record_reaches_the_row() {
        let document = DocId(Uuid::from_u128(5));
        let job = Some(Uuid::from_u128(6));
        let row = to_new_model_call(job, document, 4, &record(CallLabel::Accepted));
        assert_eq!(row.job, job);
        assert_eq!(row.document, document);
        assert_eq!(row.page, 4);
        assert_eq!((row.attempt, row.rung), (3, 1));
        assert_eq!(row.model, "claude-sonnet-5-5");
        assert_eq!(row.effort, ThinkingEffort::High);
        assert_eq!(
            row.picture,
            Picture {
                dpi: 200,
                max_edge: 2576
            }
        );
        assert_eq!(
            row.kind,
            CallKind::Escalation {
                from: 0,
                reason: EscalationReason::TotalsMismatch
            }
        );
        assert_eq!(row.prompt_version, "reading-v3/prompt-v2");
        assert_eq!(row.manifest, None);
        assert_eq!(row.usage.cache_write_1h_tokens, 500);
        assert_eq!(row.cost, Cost::from_nano_rand(19_800_000));
        assert_eq!(row.elapsed, Duration::from_millis(1_234));
        assert_eq!(row.request_id.as_deref(), Some("req_1"));
    }

    #[test]
    fn every_label_has_its_fixed_name_and_detail() {
        let cases = [
            (CallLabel::Accepted, "accepted", None),
            (
                CallLabel::Escalated(EscalationReason::TotalsMismatch),
                "escalated",
                Some("totals_mismatch"),
            ),
            (
                CallLabel::Escalated(EscalationReason::TextCheckFailed),
                "escalated",
                Some("text_check_failed"),
            ),
            (
                CallLabel::Reviewed(ReviewReason::AmountsIncludeVat),
                "reviewed",
                Some("amounts_include_vat"),
            ),
            (CallLabel::InvalidOutput, "invalid_output", None),
            (CallLabel::Truncated, "truncated", None),
            (CallLabel::Refused, "refused", None),
            (CallLabel::Rejected, "rejected", None),
            (CallLabel::RungUnavailable, "rung_unavailable", None),
            (
                CallLabel::NotSent(ReviewReason::Budget),
                "not_sent",
                Some("budget"),
            ),
            (
                CallLabel::NotSent(ReviewReason::CannotBeSentSafely),
                "not_sent",
                Some("cannot_be_sent_safely"),
            ),
            (
                CallLabel::Interrupted(FailureKind::Transient),
                "interrupted",
                Some("transient"),
            ),
            (
                CallLabel::Interrupted(FailureKind::ProviderUnavailable),
                "interrupted",
                Some("provider_unavailable"),
            ),
            (
                CallLabel::Interrupted(FailureKind::NeedsReview),
                "interrupted",
                Some("needs_review"),
            ),
        ];
        for (label, name, detail) in cases {
            let row = to_new_model_call(None, DocId(Uuid::nil()), 1, &record(label));
            assert_eq!(
                (row.label.as_str(), row.label_detail.as_deref()),
                (name, detail),
                "{label:?}"
            );
        }
    }

    #[test]
    fn the_manifest_goes_as_json_with_its_sections_and_no_text() {
        use clearcut_pack::{ImageState, PackManifest, RedactionReport, Section};
        use std::collections::BTreeMap;
        let manifest = PackManifest {
            prompt_version: "reading-v3/prompt-v2".to_string(),
            prefix_sha256: "ab".repeat(32),
            prefix_bytes: 40_000,
            schema_sha256: "cd".repeat(32),
            model: "claude-sonnet-5-5".to_string(),
            effort: "high",
            cache_ttl: "5m",
            page: 2,
            of: 3,
            text_source: "text_layer",
            sections: vec![Section {
                kind: "page_text",
                bytes: 1_200,
                cached: false,
            }],
            redaction: RedactionReport {
                policy: "best_effort",
                counts: BTreeMap::new(),
                image: ImageState::NotNeeded,
            },
            injection_suspected: Vec::new(),
            escalated_from: Some("claude-haiku-5-5".to_string()),
        };
        let mut with = record(CallLabel::Accepted);
        with.manifest = Some(manifest.clone());
        let row = to_new_model_call(None, DocId(Uuid::nil()), 2, &with);
        let json = row.manifest.expect("the manifest goes with the call");
        assert_eq!(json, serde_json::to_value(&manifest).unwrap());
        assert_eq!(json["sections"][0]["kind"], "page_text");
        assert_eq!(json["escalated_from"], "claude-haiku-5-5");
    }

    #[test]
    fn a_rung_too_large_for_a_byte_is_kept_out_of_range_for_the_store_to_refuse() {
        let mut big = record(CallLabel::Accepted);
        big.rung = 300;
        let row = to_new_model_call(None, DocId(Uuid::nil()), 1, &big);
        assert_eq!(row.rung, u8::MAX);
    }

    #[test]
    fn every_name_made_here_is_one_the_store_takes() {
        // The store's own list; a label added to the router without it would be refused there.
        let accepted = [
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
        let mut labels = vec![
            CallLabel::Accepted,
            CallLabel::InvalidOutput,
            CallLabel::Truncated,
            CallLabel::Refused,
            CallLabel::Rejected,
            CallLabel::RungUnavailable,
        ];
        labels.extend(EscalationReason::ALL.map(CallLabel::Escalated));
        labels.extend(ReviewReason::ALL.map(CallLabel::Reviewed));
        labels.extend(ReviewReason::ALL.map(CallLabel::NotSent));
        for kind in [
            FailureKind::Transient,
            FailureKind::ProviderUnavailable,
            FailureKind::NeedsReview,
        ] {
            labels.push(CallLabel::Interrupted(kind));
        }
        for label in labels {
            let (name, detail) = label_columns(label);
            assert!(accepted.contains(&name), "{label:?}");
            // And the detail is in the shape the table checks.
            if let Some(detail) = detail {
                assert!(
                    (1..=40).contains(&detail.len())
                        && detail.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                    "{detail}"
                );
            }
        }
    }
}
