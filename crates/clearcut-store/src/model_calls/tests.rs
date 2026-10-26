// Written by Kone Tshivhinda & Claude
//! The call log against a real Postgres, as the restricted `clearcut_app` role. `pool` (from
//! `sqlx::test`) is the owner connection, used to look from outside and to try what the
//! application could not do.

use super::*;
use crate::tests::{app_store, count, db_message, new_document, new_tenant, setup};
use clearcut_core::escalation::EscalationReason;
use serde_json::json;
use sqlx::PgPool;

fn call(doc: DocId, page: u32, attempt: u32) -> NewModelCall {
    NewModelCall {
        job: Some(Uuid::from_u128(77)),
        document: doc,
        page,
        attempt,
        rung: 0,
        model: "claude-haiku-5-5".to_string(),
        effort: ThinkingEffort::Medium,
        picture: Picture {
            dpi: 150,
            max_edge: 2000,
        },
        kind: CallKind::First,
        prompt_version: "reading-v2/prompt-v1".to_string(),
        manifest: Some(
            json!({ "model": "claude-haiku-5-5", "sections": [{ "kind": "rules", "bytes": 40000 }] }),
        ),
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
        label: "accepted".to_string(),
        label_detail: None,
    }
}

async fn stored(pool: &PgPool, tenant: TenantId) -> i64 {
    count(
        pool,
        &format!(
            "select count(*) from model_calls where tenant_id = '{}'",
            tenant.0
        ),
    )
    .await
}

async fn raw(pool: &PgPool, sql: String) -> sqlx::Result<sqlx::postgres::PgQueryResult> {
    sqlx::query(sqlx::AssertSqlSafe(sql)).execute(pool).await
}

const LONG_AGO: OffsetDateTime = OffsetDateTime::UNIX_EPOCH;

// ---------- recording ----------

#[sqlx::test]
async fn a_call_is_stored_with_every_field(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .record_model_calls(tenant, &[call(doc, 2, 3)])
        .await
        .unwrap();

    #[allow(clippy::type_complexity)]
    let first: (
        Uuid,
        Uuid,
        i32,
        i32,
        i16,
        String,
        String,
        i16,
        i16,
        String,
        Option<i16>,
        Option<String>,
    ) = sqlx::query_as(
        "select job_id, document_id, page, attempt, rung, model, effort, picture_dpi, picture_max_edge, \
             call_kind, escalated_from, escalation_reason from model_calls",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(first.0, Uuid::from_u128(77));
    assert_eq!(first.1, doc.0);
    assert_eq!((first.2, first.3, first.4), (2, 3, 0));
    assert_eq!(
        (first.5.as_str(), first.6.as_str()),
        ("claude-haiku-5-5", "medium")
    );
    assert_eq!((first.7, first.8), (150, 2000));
    assert_eq!((first.9.as_str(), first.10, first.11), ("first", None, None));

    #[allow(clippy::type_complexity)]
    let second: (
        String,
        Option<serde_json::Value>,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i32,
        Option<String>,
        String,
        Option<String>,
    ) = sqlx::query_as(
        "select prompt_version, manifest, input_tokens, output_tokens, cache_read_tokens, \
             cache_write_tokens, cache_write_1h_tokens, cost_nano_rand, elapsed_ms, request_id, label, \
             label_detail from model_calls",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(second.0, "reading-v2/prompt-v1");
    assert_eq!(second.1.unwrap()["model"], "claude-haiku-5-5");
    assert_eq!(
        (second.2, second.3, second.4, second.5, second.6),
        (6_000, 1_000, 10_000, 2_000, 500)
    );
    assert_eq!(second.7, 19_800_000);
    assert_eq!(second.8, 1_234);
    assert_eq!(second.9.as_deref(), Some("req_1"));
    assert_eq!((second.10.as_str(), second.11), ("accepted", None));
}

#[sqlx::test]
async fn an_escalation_names_where_it_came_from_and_why(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let kinds = [
        CallKind::First,
        CallKind::Escalation {
            from: 1,
            reason: EscalationReason::TextCheckFailed,
        },
        CallKind::RetryTruncated,
        CallKind::RetryInvalid,
    ];
    let calls: Vec<NewModelCall> = kinds
        .into_iter()
        .zip(1u32..)
        .map(|(kind, attempt)| NewModelCall {
            kind,
            ..call(doc, 1, attempt)
        })
        .collect();
    store.record_model_calls(tenant, &calls).await.unwrap();
    let rows: Vec<(String, Option<i16>, Option<String>)> = sqlx::query_as(
        "select call_kind, escalated_from, escalation_reason from model_calls order by attempt",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows,
        [
            ("first".to_string(), None, None),
            (
                "escalation".to_string(),
                Some(1),
                Some("text_check_failed".to_string())
            ),
            ("retry_truncated".to_string(), None, None),
            ("retry_invalid".to_string(), None, None),
        ]
    );
}

#[sqlx::test]
async fn a_call_that_was_not_sent_has_no_manifest_and_no_request_id(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let unsent = NewModelCall {
        manifest: None,
        request_id: None,
        usage: TokenUsage::default(),
        cost: Cost::ZERO,
        elapsed: Duration::ZERO,
        label: "not_sent".to_string(),
        label_detail: Some("cannot_be_sent_safely".to_string()),
        ..call(doc, 1, 1)
    };
    store.record_model_calls(tenant, &[unsent]).await.unwrap();
    let row: (
        Option<serde_json::Value>,
        Option<String>,
        String,
        Option<String>,
        i64,
    ) = sqlx::query_as("select manifest, request_id, label, label_detail, cost_nano_rand from model_calls")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        row,
        (
            None,
            None,
            "not_sent".to_string(),
            Some("cannot_be_sent_safely".to_string()),
            0
        )
    );
}

#[sqlx::test]
async fn every_label_the_router_uses_is_accepted(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let labels = [
        ("accepted", None),
        ("escalated", Some("totals_mismatch")),
        ("reviewed", Some("amounts_include_vat")),
        ("invalid_output", None),
        ("truncated", None),
        ("refused", None),
        ("rejected", None),
        ("rung_unavailable", None),
        ("not_sent", Some("budget")),
        ("interrupted", Some("provider_unavailable")),
    ];
    let calls: Vec<NewModelCall> = labels
        .iter()
        .zip(1u32..)
        .map(|((label, detail), attempt)| NewModelCall {
            label: label.to_string(),
            label_detail: detail.map(str::to_string),
            ..call(doc, 1, attempt)
        })
        .collect();
    store.record_model_calls(tenant, &calls).await.unwrap();
    assert_eq!(stored(&pool, tenant).await, 10);
}

#[sqlx::test]
async fn a_cost_or_a_count_too_large_for_the_column_is_stored_at_its_largest(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let huge = NewModelCall {
        cost: Cost::from_nano_rand(u64::MAX),
        usage: TokenUsage {
            input_tokens: u64::MAX,
            output_tokens: u64::MAX,
            cache_read_tokens: u64::MAX,
            cache_write_tokens: u64::MAX,
            cache_write_1h_tokens: u64::MAX,
        },
        elapsed: Duration::from_secs(u64::MAX / 2),
        ..call(doc, 1, 1)
    };
    store.record_model_calls(tenant, &[huge]).await.unwrap();
    let row: (i64, i64, i64, i64, i64, i32) = sqlx::query_as(
        "select cost_nano_rand, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, elapsed_ms \
         from model_calls",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row, (i64::MAX, i64::MAX, i64::MAX, i64::MAX, i64::MAX, i32::MAX));
}

#[sqlx::test]
async fn a_batch_is_stored_whole_or_not_at_all(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let good = call(doc, 1, 1);
    let bad = NewModelCall {
        label: "mystery".to_string(),
        ..call(doc, 1, 2)
    };
    let error = store
        .record_model_calls(tenant, &[good.clone(), bad])
        .await
        .unwrap_err();
    assert!(matches!(error, Error::InvalidModelCall(_)), "{error:?}");
    assert_eq!(stored(&pool, tenant).await, 0);

    store
        .record_model_calls(tenant, &[good, call(doc, 1, 2)])
        .await
        .unwrap();
    assert_eq!(stored(&pool, tenant).await, 2);
    // Nothing to record is not an error.
    store.record_model_calls(tenant, &[]).await.unwrap();
    assert_eq!(stored(&pool, tenant).await, 2);
}

#[sqlx::test]
async fn a_batch_may_hold_at_most_a_thousand_calls(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    assert_eq!(MAX_BATCH, 1_000);
    let many: Vec<NewModelCall> = (0..1_001).map(|_| call(doc, 1, 1)).collect();
    let error = store.record_model_calls(tenant, &many).await.unwrap_err();
    assert!(
        matches!(error, Error::InvalidModelCall("too many calls at once")),
        "{error:?}"
    );
    store.record_model_calls(tenant, &many[..1_000]).await.unwrap();
    assert_eq!(stored(&pool, tenant).await, 1_000);
}

#[sqlx::test]
async fn a_call_that_breaks_the_rules_is_refused_before_the_database(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let c = |f: &dyn Fn(&mut NewModelCall)| {
        let mut c = call(doc, 1, 1);
        f(&mut c);
        c
    };
    let cases: Vec<(NewModelCall, &str)> = vec![
        (c(&|c| c.page = 0), "pages start at 1"),
        (c(&|c| c.attempt = 0), "the attempt is outside 1 to 50"),
        (c(&|c| c.attempt = 51), "the attempt is outside 1 to 50"),
        (c(&|c| c.rung = 8), "the rung is outside the ladder"),
        (
            c(&|c| c.model = String::new()),
            "the model name is empty or too long",
        ),
        (
            c(&|c| c.model = "m".repeat(101)),
            "the model name is empty or too long",
        ),
        (
            c(&|c| c.prompt_version = String::new()),
            "the prompt version is empty or too long",
        ),
        (
            c(&|c| c.prompt_version = "v".repeat(65)),
            "the prompt version is empty or too long",
        ),
        (
            c(&|c| c.manifest = Some(json!([1]))),
            "a manifest is a JSON object of at most 15000 bytes",
        ),
        (
            c(&|c| c.manifest = Some(json!({ "x": "y".repeat(MAX_MANIFEST_BYTES) }))),
            "a manifest is a JSON object of at most 15000 bytes",
        ),
        (
            c(&|c| c.request_id = Some("r".repeat(101))),
            "a request id is at most 100 characters",
        ),
        (
            c(&|c| c.label = String::new()),
            "the label is not one the router uses",
        ),
        (
            c(&|c| c.label = "Accepted".to_string()),
            "the label is not one the router uses",
        ),
        (
            c(&|c| c.label_detail = Some("Has Capitals".to_string())),
            "a detail is lower-case words joined by underscores",
        ),
        (
            c(&|c| c.label_detail = Some(String::new())),
            "a detail is lower-case words joined by underscores",
        ),
        (
            c(&|c| c.label_detail = Some("d".repeat(41))),
            "a detail is lower-case words joined by underscores",
        ),
        (
            c(&|c| {
                c.kind = CallKind::Escalation {
                    from: 8,
                    reason: EscalationReason::LineArithmetic,
                }
            }),
            "the rung is outside the ladder",
        ),
        (
            c(&|c| c.usage.cache_write_1h_tokens = c.usage.cache_write_tokens + 1),
            "one-hour cache writes are part of the cache writes",
        ),
        (
            c(&|c| c.picture.dpi = 99),
            "the picture size is outside what the worker draws",
        ),
        (
            c(&|c| c.picture.dpi = 301),
            "the picture size is outside what the worker draws",
        ),
        (
            c(&|c| c.picture.max_edge = 499),
            "the picture size is outside what the worker draws",
        ),
        (
            c(&|c| c.picture.max_edge = 2577),
            "the picture size is outside what the worker draws",
        ),
    ];
    for (bad, expected) in cases {
        let error = store.record_model_calls(tenant, &[bad]).await.unwrap_err();
        assert!(
            matches!(&error, Error::InvalidModelCall(why) if *why == expected),
            "{expected}: {error:?}"
        );
    }
    assert_eq!(stored(&pool, tenant).await, 0);
}

#[sqlx::test]
async fn the_edges_of_every_limit_are_accepted(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let edge = NewModelCall {
        attempt: 50,
        rung: 7,
        model: "m".repeat(100),
        prompt_version: "v".repeat(64),
        request_id: Some("r".repeat(100)),
        label_detail: Some("d".repeat(40)),
        picture: Picture {
            dpi: 300,
            max_edge: 2576,
        },
        usage: TokenUsage {
            cache_write_tokens: 7,
            cache_write_1h_tokens: 7,
            ..TokenUsage::default()
        },
        ..call(doc, 1, 1)
    };
    let low = NewModelCall {
        picture: Picture {
            dpi: 100,
            max_edge: 500,
        },
        ..call(doc, 1, 1)
    };
    let from_top = NewModelCall {
        kind: CallKind::Escalation {
            from: 7,
            reason: EscalationReason::SchemaInvalid,
        },
        ..call(doc, 1, 1)
    };
    store
        .record_model_calls(tenant, &[edge, low, from_top])
        .await
        .unwrap();
    assert_eq!(stored(&pool, tenant).await, 3);
}

// ---------- the database's own checks ----------

async fn insert_raw(pool: &PgPool, tenant: TenantId, doc: DocId, columns: &str, values: &str) -> String {
    let base = "tenant_id, document_id, page, attempt, rung, model, effort, picture_dpi, picture_max_edge, \
                call_kind, prompt_version, label";
    let ok = format!(
        "'{}', '{}', 1, 1, 0, 'claude-haiku-5-5', 'medium', 150, 2000, 'first', 'p-v1', 'accepted'",
        tenant.0, doc.0
    );
    let sql = if columns.is_empty() {
        format!("insert into model_calls ({base}) values ({ok})")
    } else {
        format!("insert into model_calls ({base}, {columns}) values ({ok}, {values})")
    };
    db_message(raw(pool, sql).await)
}

#[sqlx::test]
async fn the_database_refuses_what_its_columns_forbid(pool: PgPool) {
    let (_, tenant, doc) = setup(&pool).await;
    let cases = [
        ("cost_nano_rand", "-1"),
        ("input_tokens", "-1"),
        ("cache_write_tokens, cache_write_1h_tokens", "1, 2"),
        ("elapsed_ms", "-1"),
        ("manifest", "'[]'"),
        ("label_detail", "'Not Fixed'"),
        ("escalation_reason", "'totals_mismatch'"),
        ("escalated_from", "1"),
    ];
    for (columns, values) in cases {
        let message = insert_raw(&pool, tenant, doc, columns, values).await;
        assert!(message.contains("check"), "{columns}: {message}");
    }
    // An escalation without its origin and reason; an unknown kind, effort and label.
    let sql = |kind: &str, effort: &str, label: &str| {
        format!(
            "insert into model_calls (tenant_id, document_id, page, attempt, rung, model, effort, \
             picture_dpi, picture_max_edge, call_kind, prompt_version, label) values \
             ('{}', '{}', 1, 1, 0, 'm', '{effort}', 150, 2000, '{kind}', 'p', '{label}')",
            tenant.0, doc.0
        )
    };
    for (kind, effort, label) in [
        ("escalation", "medium", "accepted"),
        ("second_opinion", "medium", "accepted"),
        ("first", "extreme", "accepted"),
        ("first", "medium", "approved"),
    ] {
        let message = db_message(raw(&pool, sql(kind, effort, label)).await);
        assert!(message.contains("check"), "{kind} {effort} {label}: {message}");
    }
    assert_eq!(stored(&pool, tenant).await, 0);
}

// ---------- append-only, privileges, tenants ----------

#[sqlx::test]
async fn the_log_cannot_be_changed_or_emptied_even_by_its_owner(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .record_model_calls(tenant, &[call(doc, 1, 1)])
        .await
        .unwrap();

    let update = db_message(raw(&pool, "update model_calls set cost_nano_rand = 0".to_string()).await);
    assert!(update.contains("append-only"), "{update}");
    let delete = db_message(raw(&pool, "delete from model_calls".to_string()).await);
    assert!(delete.contains("append-only"), "{delete}");
    // Not even with the erasure switch that events respect.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("set local clearcut.erasure = 'on'")
        .execute(&mut *tx)
        .await
        .unwrap();
    let delete = db_message(sqlx::query("delete from model_calls").execute(&mut *tx).await);
    assert!(delete.contains("append-only"), "{delete}");
    assert_eq!(stored(&pool, tenant).await, 1);
}

#[sqlx::test]
async fn the_privileges_are_the_ones_the_migration_gives(pool: PgPool) {
    let _ = app_store(&pool).await; // runs the migrations
    for (grantee, expected) in [
        ("clearcut_app", vec!["INSERT", "SELECT"]),
        ("clearcut_eraser", vec![]),
    ] {
        let rows: Vec<(String,)> = sqlx::query_as(
            "select privilege_type::text from information_schema.role_table_grants \
             where table_name = 'model_calls' and grantee = $1 order by 1",
        )
        .bind(grantee)
        .fetch_all(&pool)
        .await
        .unwrap();
        let names: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(names, expected, "{grantee}");
    }
}

#[sqlx::test]
async fn one_tenant_cannot_see_or_write_another_tenants_calls(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Advisory").await.unwrap();
    let theirs = store.add_document(other, &new_document(9)).await.unwrap().id;
    store
        .record_model_calls(tenant, &[call(doc, 1, 1), call(doc, 1, 2)])
        .await
        .unwrap();
    store
        .record_model_calls(other, &[call(theirs, 1, 1)])
        .await
        .unwrap();

    let mut tx = store.begin(tenant).await.unwrap();
    let seen: i64 = sqlx::query_scalar("select count(*) from model_calls")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(seen, 2);
    // A row that says it belongs to someone else is refused by the policy.
    let result = sqlx::query(sqlx::AssertSqlSafe(format!(
        "insert into model_calls (tenant_id, document_id, page, attempt, rung, model, effort, \
         picture_dpi, picture_max_edge, call_kind, prompt_version, label) values \
         ('{}', '{}', 1, 1, 0, 'm', 'low', 150, 2000, 'first', 'p', 'accepted')",
        other.0, theirs.0
    )))
    .execute(&mut *tx)
    .await;
    let message = db_message(result);
    assert!(
        message.contains("row-level security") || message.contains("policy"),
        "{message}"
    );
}

// ---------- erasure ----------

#[sqlx::test]
async fn erasing_a_document_keeps_what_it_cost(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .record_model_calls(tenant, &[call(doc, 1, 1)])
        .await
        .unwrap();
    store.erase_document(tenant, doc).await.unwrap();
    assert_eq!(stored(&pool, tenant).await, 1);
    assert_eq!(
        store.spend_since(tenant, LONG_AGO).await.unwrap(),
        Cost::from_nano_rand(19_800_000)
    );
}

#[sqlx::test]
async fn a_call_can_be_recorded_for_a_document_that_has_since_been_erased(pool: PgPool) {
    // Spending that happened is recorded whatever became of the document.
    let (store, tenant, doc) = setup(&pool).await;
    store.erase_document(tenant, doc).await.unwrap();
    store
        .record_model_calls(tenant, &[call(doc, 1, 1)])
        .await
        .unwrap();
    assert_eq!(stored(&pool, tenant).await, 1);
}

// ---------- adding it up ----------

#[sqlx::test]
async fn spend_is_the_sum_of_what_was_charged_since_a_time_for_one_tenant(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Advisory").await.unwrap();
    let theirs = store.add_document(other, &new_document(9)).await.unwrap().id;
    assert_eq!(store.spend_since(tenant, LONG_AGO).await.unwrap(), Cost::ZERO);

    let priced = |nano: u64, attempt: u32| NewModelCall {
        cost: Cost::from_nano_rand(nano),
        ..call(doc, 1, attempt)
    };
    store
        .record_model_calls(tenant, &[priced(100, 1), priced(250, 2), priced(0, 3)])
        .await
        .unwrap();
    store
        .record_model_calls(
            other,
            &[NewModelCall {
                cost: Cost::from_nano_rand(9_999),
                ..call(theirs, 1, 1)
            }],
        )
        .await
        .unwrap();
    assert_eq!(
        store.spend_since(tenant, LONG_AGO).await.unwrap(),
        Cost::from_nano_rand(350)
    );
    assert_eq!(
        store.spend_since(other, LONG_AGO).await.unwrap(),
        Cost::from_nano_rand(9_999)
    );
}

#[sqlx::test]
async fn spend_counts_only_calls_made_at_or_after_the_time_given(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let priced = |nano: u64, attempt: u32| NewModelCall {
        cost: Cost::from_nano_rand(nano),
        ..call(doc, 1, attempt)
    };
    store
        .record_model_calls(tenant, &[priced(100, 1), priced(200, 2)])
        .await
        .unwrap();
    // Move the first call into last month (the owner may; the application may not).
    sqlx::query("alter table model_calls disable trigger model_calls_append_only")
        .execute(&pool)
        .await
        .unwrap();
    raw(
        &pool,
        "update model_calls set created_at = now() - interval '40 days' where attempt = 1".to_string(),
    )
    .await
    .unwrap();
    sqlx::query("alter table model_calls enable trigger model_calls_append_only")
        .execute(&pool)
        .await
        .unwrap();

    let a_week_ago = OffsetDateTime::now_utc() - time::Duration::days(7);
    assert_eq!(
        store.spend_since(tenant, a_week_ago).await.unwrap(),
        Cost::from_nano_rand(200)
    );
    assert_eq!(
        store.spend_since(tenant, LONG_AGO).await.unwrap(),
        Cost::from_nano_rand(300)
    );
    let tomorrow = OffsetDateTime::now_utc() + time::Duration::days(1);
    assert_eq!(store.spend_since(tenant, tomorrow).await.unwrap(), Cost::ZERO);
}

#[sqlx::test]
async fn a_sum_too_large_for_a_cost_is_the_largest_cost(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let huge = |attempt: u32| NewModelCall {
        cost: Cost::from_nano_rand(u64::MAX),
        ..call(doc, 1, attempt)
    };
    // Each is stored at i64::MAX; three of them are more than a u64 holds.
    store
        .record_model_calls(tenant, &[huge(1), huge(2), huge(3)])
        .await
        .unwrap();
    assert_eq!(
        store.spend_since(tenant, LONG_AGO).await.unwrap(),
        Cost::from_nano_rand(u64::MAX)
    );
}

#[sqlx::test]
async fn a_manifest_may_be_exactly_as_large_as_the_limit_and_no_larger(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    // {"x":""} is 8 bytes of JSON around the text.
    let manifest = |filler: usize| json!({ "x": "y".repeat(filler) });
    assert_eq!(
        serde_json::to_string(&manifest(MAX_MANIFEST_BYTES - 8))
            .unwrap()
            .len(),
        MAX_MANIFEST_BYTES
    );
    let with = |filler: usize| NewModelCall {
        manifest: Some(manifest(filler)),
        ..call(doc, 1, 1)
    };
    store
        .record_model_calls(tenant, &[with(MAX_MANIFEST_BYTES - 8)])
        .await
        .unwrap();
    let error = store
        .record_model_calls(tenant, &[with(MAX_MANIFEST_BYTES - 7)])
        .await
        .unwrap_err();
    assert!(matches!(error, Error::InvalidModelCall(_)), "{error:?}");
}

#[sqlx::test]
async fn every_effort_is_stored_under_its_own_name(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let calls: Vec<NewModelCall> = [ThinkingEffort::Low, ThinkingEffort::Medium, ThinkingEffort::High]
        .into_iter()
        .zip(1u32..)
        .map(|(effort, attempt)| NewModelCall {
            effort,
            ..call(doc, 1, attempt)
        })
        .collect();
    store.record_model_calls(tenant, &calls).await.unwrap();
    let efforts: Vec<(String,)> = sqlx::query_as("select effort from model_calls order by attempt")
        .fetch_all(&pool)
        .await
        .unwrap();
    let efforts: Vec<&str> = efforts.iter().map(|e| e.0.as_str()).collect();
    assert_eq!(efforts, ["low", "medium", "high"]);
}

#[sqlx::test]
async fn spend_includes_a_call_made_at_exactly_the_time_given(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .record_model_calls(tenant, &[call(doc, 1, 1)])
        .await
        .unwrap();
    let at: OffsetDateTime = sqlx::query_scalar("select created_at from model_calls")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        store.spend_since(tenant, at).await.unwrap(),
        Cost::from_nano_rand(19_800_000)
    );
    let just_after = at + time::Duration::microseconds(1);
    assert_eq!(store.spend_since(tenant, just_after).await.unwrap(), Cost::ZERO);
}

#[sqlx::test]
async fn a_detail_is_lower_case_letters_and_underscores_only(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    for bad in ["HasCapitals", "has1digit", "has-dash", "has space", "übung"] {
        let call = NewModelCall {
            label_detail: Some(bad.to_string()),
            ..call(doc, 1, 1)
        };
        let error = store.record_model_calls(tenant, &[call]).await.unwrap_err();
        assert!(
            matches!(
                &error,
                Error::InvalidModelCall("a detail is lower-case words joined by underscores")
            ),
            "{bad}: {error:?}"
        );
    }
    let fine = NewModelCall {
        label_detail: Some("a_b_c".to_string()),
        ..call(doc, 1, 1)
    };
    store.record_model_calls(tenant, &[fine]).await.unwrap();
}

// ---------- what one document has cost ----------

#[sqlx::test]
async fn a_documents_spend_is_what_it_was_charged_and_the_calls_that_were_made(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other_doc = store.add_document(tenant, &new_document(5)).await.unwrap().id;
    let other_tenant = new_tenant(&pool, "Other Advisory").await.unwrap();
    let theirs = store
        .add_document(other_tenant, &new_document(9))
        .await
        .unwrap()
        .id;
    assert_eq!(
        store.document_spend(tenant, doc).await.unwrap(),
        DocumentSpend {
            cost: Cost::ZERO,
            calls: 0
        }
    );

    let with = |document: DocId, attempt: u32, nano: u64, label: &str| NewModelCall {
        cost: Cost::from_nano_rand(nano),
        label: label.to_string(),
        label_detail: matches!(label, "not_sent" | "interrupted" | "escalated").then(|| "budget".to_string()),
        ..call(document, 1, attempt)
    };
    store
        .record_model_calls(
            tenant,
            &[
                with(doc, 1, 100, "escalated"),
                with(doc, 2, 250, "accepted"),
                with(doc, 3, 0, "rung_unavailable"),
                // A call that was refused before it was sent, and one the service did not answer,
                // are not calls the page was allowed (the state machine does not count them), but
                // an interrupted one may have been charged.
                with(doc, 4, 0, "not_sent"),
                with(doc, 5, 40, "interrupted"),
                with(other_doc, 1, 7_000, "accepted"),
            ],
        )
        .await
        .unwrap();
    store
        .record_model_calls(other_tenant, &[with(theirs, 1, 9_999, "accepted")])
        .await
        .unwrap();

    assert_eq!(
        store.document_spend(tenant, doc).await.unwrap(),
        DocumentSpend {
            cost: Cost::from_nano_rand(390),
            calls: 3
        }
    );
    assert_eq!(
        store.document_spend(tenant, other_doc).await.unwrap(),
        DocumentSpend {
            cost: Cost::from_nano_rand(7_000),
            calls: 1
        }
    );
    // Another tenant's document is not visible, even by its id.
    assert_eq!(
        store.document_spend(tenant, theirs).await.unwrap(),
        DocumentSpend {
            cost: Cost::ZERO,
            calls: 0
        }
    );
}

#[sqlx::test]
async fn a_documents_spend_outlives_the_document(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .record_model_calls(tenant, &[call(doc, 1, 1)])
        .await
        .unwrap();
    store.erase_document(tenant, doc).await.unwrap();
    assert_eq!(
        store.document_spend(tenant, doc).await.unwrap(),
        DocumentSpend {
            cost: Cost::from_nano_rand(19_800_000),
            calls: 1
        }
    );
}

#[sqlx::test]
async fn a_documents_spend_saturates_instead_of_wrapping(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let huge = |attempt: u32| NewModelCall {
        cost: Cost::from_nano_rand(u64::MAX),
        ..call(doc, 1, attempt)
    };
    store
        .record_model_calls(tenant, &[huge(1), huge(2), huge(3)])
        .await
        .unwrap();
    let spend = store.document_spend(tenant, doc).await.unwrap();
    assert_eq!(spend.cost, Cost::from_nano_rand(u64::MAX));
    assert_eq!(spend.calls, 3);
}
