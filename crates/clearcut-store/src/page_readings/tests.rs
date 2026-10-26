// Written by Kone Tshivhinda & Claude
//! Page readings against a real Postgres, as the restricted `clearcut_app` role the service
//! runs as. `pool` (from `sqlx::test`) is the owner connection: tests use it to look from
//! outside and to try rows the application could not write.

use super::*;
use crate::tests::{app_store, count, db_message, new_document, new_tenant, setup};
use serde_json::json;
use sqlx::PgPool;

fn carry() -> Value {
    json!({ "machine": { "opaque": "to the store" }, "best": null, "last": null })
}

fn kept(rung: u8, attempt: u32, confidence: ConfidenceBasis) -> KeptReading {
    KeptReading {
        rung,
        attempt,
        confidence,
        model: "claude-haiku-5-5".to_string(),
        reading: json!({ "invoice_number": "INV-1", "lines": [] }),
    }
}

fn details() -> PageDetails {
    PageDetails {
        problems: json!([{ "path": "invoice_date", "kind": "bad_date" }]),
        findings: json!([{ "type": "due_before_issue" }]),
        notes: json!([{ "type": "due_before_issue" }]),
        unconfirmed: vec!["totals/gross".to_string()],
        injection_suspected: vec!["ignore_previous".to_string()],
    }
}

async fn stored_count(pool: &PgPool, doc: DocId) -> i64 {
    count(
        pool,
        &format!(
            "select count(*) from page_readings where document_id = '{}'",
            doc.0
        ),
    )
    .await
}

async fn raw(pool: &PgPool, sql: String) -> sqlx::Result<sqlx::postgres::PgQueryResult> {
    sqlx::query(sqlx::AssertSqlSafe(sql)).execute(pool).await
}

/// A raw insert of a row with only the columns given (everything else at its default).
async fn insert_raw(
    pool: &PgPool,
    doc: DocId,
    tenant: TenantId,
    page: u32,
    columns: &str,
    values: &str,
) -> String {
    let sql = format!(
        "insert into page_readings (tenant_id, document_id, page, {columns}) values ('{}', '{}', {page}, {values})",
        tenant.0, doc.0
    );
    db_message(raw(pool, sql).await)
}

// ---------- storing and loading ----------

#[sqlx::test]
async fn a_page_in_progress_comes_back_with_its_carry(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let state = PageState::InProgress { carry: carry() };
    store
        .save_page_reading(tenant, doc, 1, &state, &PageDetails::default())
        .await
        .unwrap();
    let loaded = store.load_page_readings(tenant, doc).await.unwrap();
    assert_eq!(
        loaded,
        [PageReading {
            page: 1,
            state,
            details: PageDetails::default()
        }]
    );
}

#[sqlx::test]
async fn a_page_that_was_read_comes_back_with_its_reading_and_details(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let state = PageState::Read(kept(1, 2, ConfidenceBasis::TextConfirmed));
    store
        .save_page_reading(tenant, doc, 1, &state, &details())
        .await
        .unwrap();
    let loaded = store.load_page_readings(tenant, doc).await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].state, state);
    assert_eq!(loaded[0].details, details());
}

#[sqlx::test]
async fn a_page_for_a_person_keeps_its_best_draft_or_none(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let with_draft = PageState::NeedsReview {
        reason: ReviewReason::TopRungFailed,
        draft: Some(kept(2, 3, ConfidenceBasis::Unresolved)),
    };
    let without = PageState::NeedsReview {
        reason: ReviewReason::CannotBeSentSafely,
        draft: None,
    };
    store
        .save_page_reading(tenant, doc, 1, &with_draft, &details())
        .await
        .unwrap();
    store
        .save_page_reading(tenant, doc, 2, &without, &PageDetails::default())
        .await
        .unwrap();
    let loaded = store.load_page_readings(tenant, doc).await.unwrap();
    assert_eq!(loaded[0].state, with_draft);
    assert_eq!(loaded[1].state, without);
}

#[sqlx::test]
async fn every_review_reason_and_every_basis_survives_the_round_trip(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    for (i, reason) in ReviewReason::ALL.into_iter().enumerate() {
        let page = u32::try_from(i + 1).unwrap();
        let state = PageState::NeedsReview { reason, draft: None };
        store
            .save_page_reading(tenant, doc, page, &state, &PageDetails::default())
            .await
            .unwrap();
    }
    let loaded = store.load_page_readings(tenant, doc).await.unwrap();
    let reasons: Vec<ReviewReason> = loaded
        .iter()
        .map(|p| match &p.state {
            PageState::NeedsReview { reason, .. } => *reason,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(reasons, ReviewReason::ALL);

    for (offset, basis) in [ConfidenceBasis::TextConfirmed, ConfidenceBasis::ArithmeticOnly]
        .into_iter()
        .enumerate()
    {
        let page = 20 + u32::try_from(offset).unwrap();
        let state = PageState::Read(kept(0, 1, basis));
        store
            .save_page_reading(tenant, doc, page, &state, &PageDetails::default())
            .await
            .unwrap();
    }
    let loaded = store.load_page_readings(tenant, doc).await.unwrap();
    assert!(
        matches!(&loaded[ReviewReason::ALL.len()].state, PageState::Read(k) if k.confidence == ConfidenceBasis::TextConfirmed)
    );
    assert!(
        matches!(&loaded[ReviewReason::ALL.len() + 1].state, PageState::Read(k) if k.confidence == ConfidenceBasis::ArithmeticOnly)
    );
}

#[sqlx::test]
async fn pages_come_back_in_page_order(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    for page in [3, 1, 2] {
        store
            .save_page_reading(
                tenant,
                doc,
                page,
                &PageState::InProgress { carry: carry() },
                &PageDetails::default(),
            )
            .await
            .unwrap();
    }
    let pages: Vec<u32> = store
        .load_page_readings(tenant, doc)
        .await
        .unwrap()
        .iter()
        .map(|p| p.page)
        .collect();
    assert_eq!(pages, [1, 2, 3]);
}

#[sqlx::test]
async fn a_document_with_no_readings_loads_as_an_empty_list(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    assert_eq!(store.load_page_readings(tenant, doc).await.unwrap(), Vec::new());
}

// ---------- a page in progress becomes a page that ended, once ----------

#[sqlx::test]
async fn a_page_in_progress_is_replaced_by_each_save_and_then_by_its_end(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let first = json!({ "machine": 1, "best": null, "last": null });
    let second = json!({ "machine": 2, "best": null, "last": null });
    for carry in [&first, &second] {
        store
            .save_page_reading(
                tenant,
                doc,
                1,
                &PageState::InProgress { carry: carry.clone() },
                &PageDetails::default(),
            )
            .await
            .unwrap();
    }
    assert_eq!(stored_count(&pool, doc).await, 1);
    let loaded = store.load_page_readings(tenant, doc).await.unwrap();
    assert_eq!(loaded[0].state, PageState::InProgress { carry: second });

    let end = PageState::Read(kept(0, 3, ConfidenceBasis::TextConfirmed));
    store
        .save_page_reading(tenant, doc, 1, &end, &details())
        .await
        .unwrap();
    let loaded = store.load_page_readings(tenant, doc).await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].state, end);
    // The carry is gone: it held readings that are now the page's.
    let carries = count(
        &pool,
        &format!(
            "select count(*) from page_readings where document_id = '{}' and carry is not null",
            doc.0
        ),
    )
    .await;
    assert_eq!(carries, 0);
}

#[sqlx::test]
async fn a_page_that_ended_is_final_until_its_rows_are_deleted(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let end = PageState::Read(kept(0, 1, ConfidenceBasis::TextConfirmed));
    store
        .save_page_reading(tenant, doc, 1, &end, &details())
        .await
        .unwrap();

    // Neither a stale writer's carry nor a second ending replaces it.
    for state in [
        PageState::InProgress { carry: carry() },
        PageState::Read(kept(2, 4, ConfidenceBasis::ArithmeticOnly)),
        PageState::NeedsReview {
            reason: ReviewReason::Budget,
            draft: None,
        },
    ] {
        let error = store
            .save_page_reading(tenant, doc, 1, &state, &PageDetails::default())
            .await
            .unwrap_err();
        assert!(matches!(error, Error::PageReadingFinished), "{error:?}");
    }
    let loaded = store.load_page_readings(tenant, doc).await.unwrap();
    assert_eq!(loaded[0].state, end);
    assert_eq!(loaded[0].details, details());

    // A re-run deletes first, and then the page can be read again.
    assert_eq!(store.delete_page_readings(tenant, doc).await.unwrap(), 1);
    store
        .save_page_reading(
            tenant,
            doc,
            1,
            &PageState::InProgress { carry: carry() },
            &PageDetails::default(),
        )
        .await
        .unwrap();
}

#[sqlx::test]
async fn deleting_readings_removes_them_all_and_counts_them(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    assert_eq!(store.delete_page_readings(tenant, doc).await.unwrap(), 0);
    for page in 1..=3 {
        store
            .save_page_reading(
                tenant,
                doc,
                page,
                &PageState::InProgress { carry: carry() },
                &PageDetails::default(),
            )
            .await
            .unwrap();
    }
    assert_eq!(store.delete_page_readings(tenant, doc).await.unwrap(), 3);
    assert_eq!(stored_count(&pool, doc).await, 0);
}

#[sqlx::test]
async fn the_time_of_the_last_change_moves_and_the_time_of_the_first_does_not(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let save = |carry: Value| {
        let store = store.clone();
        async move {
            store
                .save_page_reading(
                    tenant,
                    doc,
                    1,
                    &PageState::InProgress { carry },
                    &PageDetails::default(),
                )
                .await
                .unwrap();
        }
    };
    save(carry()).await;
    let first: (time::OffsetDateTime, time::OffsetDateTime) =
        sqlx::query_as("select created_at, updated_at from page_readings where page = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("select pg_sleep(0.05)").execute(&pool).await.unwrap();
    save(json!({ "machine": 9, "best": null, "last": null })).await;
    let second: (time::OffsetDateTime, time::OffsetDateTime) =
        sqlx::query_as("select created_at, updated_at from page_readings where page = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(second.0, first.0);
    assert!(second.1 > first.1);
}

// ---------- what is refused before anything is written ----------

#[sqlx::test]
async fn a_state_that_contradicts_itself_is_refused_before_the_database(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let long_name = "m".repeat(101);
    let huge = json!({ "x": "y".repeat(MAX_JSON_BYTES) });
    let cases: Vec<(PageState, PageDetails, &str)> = vec![
        (
            PageState::Read(kept(0, 1, ConfidenceBasis::Unresolved)),
            PageDetails::default(),
            "a page that was read has evidence",
        ),
        (
            PageState::NeedsReview {
                reason: ReviewReason::TopRungFailed,
                draft: Some(kept(0, 1, ConfidenceBasis::TextConfirmed)),
            },
            PageDetails::default(),
            "a draft is unresolved",
        ),
        (
            PageState::Read(kept(8, 1, ConfidenceBasis::TextConfirmed)),
            PageDetails::default(),
            "the rung is outside the ladder",
        ),
        (
            PageState::Read(kept(0, 0, ConfidenceBasis::TextConfirmed)),
            PageDetails::default(),
            "the attempt is outside 1 to 50",
        ),
        (
            PageState::Read(kept(0, 51, ConfidenceBasis::TextConfirmed)),
            PageDetails::default(),
            "the attempt is outside 1 to 50",
        ),
        (
            PageState::Read(KeptReading {
                model: String::new(),
                ..kept(0, 1, ConfidenceBasis::TextConfirmed)
            }),
            PageDetails::default(),
            "the model name is empty or too long",
        ),
        (
            PageState::Read(KeptReading {
                model: long_name,
                ..kept(0, 1, ConfidenceBasis::TextConfirmed)
            }),
            PageDetails::default(),
            "the model name is empty or too long",
        ),
        (
            PageState::Read(KeptReading {
                reading: json!([1, 2]),
                ..kept(0, 1, ConfidenceBasis::TextConfirmed)
            }),
            PageDetails::default(),
            "a reading is a JSON object",
        ),
        (
            PageState::Read(KeptReading {
                reading: huge.clone(),
                ..kept(0, 1, ConfidenceBasis::TextConfirmed)
            }),
            PageDetails::default(),
            "a reading or a carry is too large",
        ),
        (
            PageState::InProgress { carry: json!("text") },
            PageDetails::default(),
            "a carry is a JSON object",
        ),
        (
            PageState::InProgress { carry: huge },
            PageDetails::default(),
            "a reading or a carry is too large",
        ),
        (
            PageState::InProgress { carry: carry() },
            PageDetails {
                problems: json!({}),
                ..PageDetails::default()
            },
            "problems, findings and notes are lists",
        ),
        (
            PageState::InProgress { carry: carry() },
            PageDetails {
                notes: json!("x"),
                ..PageDetails::default()
            },
            "problems, findings and notes are lists",
        ),
        (
            PageState::InProgress { carry: carry() },
            PageDetails {
                findings: json!(["x".repeat(MAX_JSON_BYTES)]),
                ..PageDetails::default()
            },
            "problems, findings and notes are lists",
        ),
        (
            PageState::InProgress { carry: carry() },
            PageDetails {
                unconfirmed: vec!["p".to_string(); 501],
                ..PageDetails::default()
            },
            "too many places or kinds are listed",
        ),
        (
            PageState::InProgress { carry: carry() },
            PageDetails {
                injection_suspected: vec!["k".to_string(); 9],
                ..PageDetails::default()
            },
            "too many places or kinds are listed",
        ),
        (
            PageState::InProgress { carry: carry() },
            PageDetails {
                unconfirmed: vec!["p".repeat(101)],
                ..PageDetails::default()
            },
            "a place or a kind is empty or too long",
        ),
        (
            PageState::InProgress { carry: carry() },
            PageDetails {
                injection_suspected: vec![String::new()],
                ..PageDetails::default()
            },
            "a place or a kind is empty or too long",
        ),
    ];
    for (state, details, expected) in cases {
        let error = store
            .save_page_reading(tenant, doc, 1, &state, &details)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::InvalidPageReading(why) if *why == expected),
            "{expected}: {error:?}"
        );
    }
    let error = store
        .save_page_reading(
            tenant,
            doc,
            0,
            &PageState::InProgress { carry: carry() },
            &PageDetails::default(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::InvalidPageReading("pages start at 1")),
        "{error:?}"
    );
    assert_eq!(stored_count(&pool, doc).await, 0);
}

#[sqlx::test]
async fn the_edges_of_every_limit_are_accepted(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let edge = PageState::Read(KeptReading {
        rung: 7,
        attempt: 50,
        model: "m".repeat(100),
        ..kept(7, 50, ConfidenceBasis::ArithmeticOnly)
    });
    let details = PageDetails {
        unconfirmed: vec!["p".repeat(100); 500],
        injection_suspected: vec!["k".repeat(100); 8],
        ..PageDetails::default()
    };
    store
        .save_page_reading(tenant, doc, 1, &edge, &details)
        .await
        .unwrap();
    let low = PageState::Read(kept(0, 1, ConfidenceBasis::TextConfirmed));
    store
        .save_page_reading(tenant, doc, 2, &low, &PageDetails::default())
        .await
        .unwrap();
    assert_eq!(store.load_page_readings(tenant, doc).await.unwrap().len(), 2);
}

#[sqlx::test]
async fn a_document_that_is_not_yours_is_not_found(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Advisory").await.unwrap();
    let theirs = store.add_document(other, &new_document(9)).await.unwrap().id;

    let state = PageState::InProgress { carry: carry() };
    let error = store
        .save_page_reading(tenant, theirs, 1, &state, &PageDetails::default())
        .await
        .unwrap_err();
    assert!(matches!(error, Error::DocumentNotFound), "{error:?}");
    let error = store.load_page_readings(tenant, theirs).await.unwrap_err();
    assert!(matches!(error, Error::DocumentNotFound), "{error:?}");
    let error = store.delete_page_readings(tenant, theirs).await.unwrap_err();
    assert!(matches!(error, Error::DocumentNotFound), "{error:?}");

    // And the other way round, with a document that exists.
    store
        .save_page_reading(tenant, doc, 1, &state, &PageDetails::default())
        .await
        .unwrap();
    let error = store.load_page_readings(other, doc).await.unwrap_err();
    assert!(matches!(error, Error::DocumentNotFound), "{error:?}");
    assert_eq!(stored_count(&pool, theirs).await, 0);
}

// ---------- the database's own checks ----------

#[sqlx::test]
async fn the_database_refuses_a_row_whose_columns_do_not_fit_its_state(pool: PgPool) {
    let (_, tenant, doc) = setup(&pool).await;
    let cases = [
        // In progress without a carry; with a reading.
        ("state", "'in_progress'"),
        ("state, carry, reading", "'in_progress', '{}', '{}'"),
        // Read without a reading; with a review reason; with unresolved evidence; with no model.
        ("state", "'read'"),
        (
            "state, reading, rung, attempt, model, confidence_basis, review_reason",
            "'read', '{}', 0, 1, 'm', 'text_confirmed', 'budget'",
        ),
        (
            "state, reading, rung, attempt, model, confidence_basis",
            "'read', '{}', 0, 1, 'm', 'unresolved'",
        ),
        (
            "state, reading, rung, attempt, confidence_basis",
            "'read', '{}', 0, 1, 'text_confirmed'",
        ),
        // Needs review without a reason; with a half-given draft; with a draft that is confident.
        ("state", "'needs_review'"),
        ("state, review_reason, reading", "'needs_review', 'budget', '{}'"),
        (
            "state, review_reason, reading, rung, attempt, model, confidence_basis",
            "'needs_review', 'budget', '{}', 0, 1, 'm', 'text_confirmed'",
        ),
        ("state, review_reason, rung", "'needs_review', 'budget', 0"),
        // A state that does not exist; a reason that does not exist.
        ("state", "'done'"),
        ("state, review_reason", "'needs_review', 'because'"),
        // A carry on a page that ended.
        ("state, review_reason, carry", "'needs_review', 'budget', '{}'"),
    ];
    for (i, (columns, values)) in cases.into_iter().enumerate() {
        let message = insert_raw(&pool, doc, tenant, 1, columns, values).await;
        assert!(message.contains("check"), "case {i}: {message}");
    }
    assert_eq!(stored_count(&pool, doc).await, 0);
}

#[sqlx::test]
async fn the_database_refuses_page_zero_and_values_out_of_range(pool: PgPool) {
    let (_, tenant, doc) = setup(&pool).await;
    let ok = "'needs_review', 'budget'";
    let m = insert_raw(&pool, doc, tenant, 0, "state, review_reason", ok).await;
    assert!(m.contains("check"), "{m}");
    let ready = "state, reading, rung, attempt, model, confidence_basis";
    for values in [
        "'read', '{}', 8, 1, 'm', 'text_confirmed'",
        "'read', '{}', -1, 1, 'm', 'text_confirmed'",
        "'read', '{}', 0, 0, 'm', 'text_confirmed'",
        "'read', '{}', 0, 51, 'm', 'text_confirmed'",
        "'read', '{}', 0, 1, '', 'text_confirmed'",
        "'read', '{}', 0, 1, 'm', 'certain'",
    ] {
        let m = insert_raw(&pool, doc, tenant, 1, ready, values).await;
        assert!(m.contains("check"), "{values}: {m}");
    }
    let m = insert_raw(
        &pool,
        doc,
        tenant,
        1,
        "state, carry, problems",
        "'in_progress', '{}', '{}'",
    )
    .await;
    assert!(m.contains("check"), "{m}");
    let m = insert_raw(
        &pool,
        doc,
        tenant,
        1,
        "state, carry, injection_suspected",
        "'in_progress', '{}', array['a','b','c','d','e','f','g','h','i']",
    )
    .await;
    assert!(m.contains("check"), "{m}");
}

#[sqlx::test]
async fn a_page_has_one_row_and_a_row_needs_a_document_of_its_own_tenant(pool: PgPool) {
    let (_, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Advisory").await.unwrap();
    let insert = |tenant: TenantId, page: u32| {
        format!(
            "insert into page_readings (tenant_id, document_id, page, state, review_reason)              values ('{}', '{}', {page}, 'needs_review', 'budget')",
            tenant.0, doc.0
        )
    };
    raw(&pool, insert(tenant, 1)).await.unwrap();
    // A second row for the same page, however it differs.
    let m = db_message(raw(&pool, insert(tenant, 1)).await);
    assert!(m.contains("duplicate key") || m.contains("unique"), "{m}");
    // A row that says it belongs to another tenant than the document does.
    let m = db_message(raw(&pool, insert(other, 2)).await);
    assert!(m.contains("foreign key"), "{m}");
}

// ---------- tenants and privileges ----------

#[sqlx::test]
async fn one_tenants_readings_are_invisible_and_untouchable_to_another(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Advisory").await.unwrap();
    let theirs = store.add_document(other, &new_document(9)).await.unwrap().id;
    let state = PageState::InProgress { carry: carry() };
    for (t, d) in [(tenant, doc), (other, theirs)] {
        for page in 1..=2 {
            store
                .save_page_reading(t, d, page, &state, &PageDetails::default())
                .await
                .unwrap();
        }
    }
    // A statement with no where clause, acting for the first tenant, sees and changes only its own.
    let mut tx = store.begin(tenant).await.unwrap();
    let seen: i64 = sqlx::query_scalar("select count(*) from page_readings")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(seen, 2);
    let changed = sqlx::query("update page_readings set updated_at = now()")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(changed.rows_affected(), 2);
    let removed = sqlx::query("delete from page_readings")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert_eq!(removed.rows_affected(), 2);
    tx.commit().await.unwrap();
    assert_eq!(stored_count(&pool, theirs).await, 2);
}

#[sqlx::test]
async fn the_application_cannot_write_a_row_for_another_tenant(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Advisory").await.unwrap();
    let mut tx = store.begin(other).await.unwrap();
    let result = sqlx::query(sqlx::AssertSqlSafe(format!(
        "insert into page_readings (tenant_id, document_id, page, state, carry) \
         values ('{}', '{}', 1, 'in_progress', '{{}}')",
        tenant.0, doc.0
    )))
    .execute(&mut *tx)
    .await;
    let message = db_message(result);
    assert!(
        message.contains("row-level security") || message.contains("policy"),
        "{message}"
    );
}

#[sqlx::test]
async fn the_privileges_are_the_ones_the_migration_gives(pool: PgPool) {
    let _ = app_store(&pool).await; // runs the migrations
    for (grantee, expected) in [
        ("clearcut_app", vec!["DELETE", "INSERT", "SELECT", "UPDATE"]),
        ("clearcut_eraser", vec!["DELETE", "SELECT"]),
    ] {
        let rows: Vec<(String,)> = sqlx::query_as(
            "select privilege_type::text from information_schema.role_table_grants \
             where table_name = 'page_readings' and grantee = $1 order by 1",
        )
        .bind(grantee)
        .fetch_all(&pool)
        .await
        .unwrap();
        let names: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(names, expected, "{grantee}");
    }
}

// ---------- erasure ----------

#[sqlx::test]
async fn erasing_a_document_takes_its_readings_and_leaves_the_others(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let theirs = store.add_document(tenant, &new_document(9)).await.unwrap().id;
    let state = PageState::Read(kept(0, 1, ConfidenceBasis::TextConfirmed));
    for d in [doc, theirs] {
        store
            .save_page_reading(tenant, d, 1, &state, &details())
            .await
            .unwrap();
    }
    store.erase_document(tenant, doc).await.unwrap();
    assert_eq!(stored_count(&pool, doc).await, 0);
    assert_eq!(stored_count(&pool, theirs).await, 1);
}

// ---------- the exact limits, and rows the database alone would let through ----------

#[sqlx::test]
async fn a_reading_may_be_exactly_as_large_as_the_limit_and_no_larger(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    // {"x":""} is 8 bytes of JSON around the text.
    let reading = |filler: usize| json!({ "x": "y".repeat(filler) });
    assert_eq!(
        serde_json::to_string(&reading(MAX_JSON_BYTES - 8)).unwrap().len(),
        MAX_JSON_BYTES
    );
    let state = |filler: usize| {
        PageState::Read(KeptReading {
            reading: reading(filler),
            ..kept(0, 1, ConfidenceBasis::TextConfirmed)
        })
    };
    store
        .save_page_reading(
            tenant,
            doc,
            1,
            &state(MAX_JSON_BYTES - 8),
            &PageDetails::default(),
        )
        .await
        .unwrap();
    let error = store
        .save_page_reading(
            tenant,
            doc,
            2,
            &state(MAX_JSON_BYTES - 7),
            &PageDetails::default(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::InvalidPageReading("a reading or a carry is too large")
        ),
        "{error:?}"
    );
}

#[sqlx::test]
async fn problems_findings_and_notes_may_be_exactly_as_large_as_the_limit_and_no_larger(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    // ["..."] is 4 bytes of JSON around the text.
    let list = |filler: usize| json!(["y".repeat(filler)]);
    assert_eq!(
        serde_json::to_string(&list(MAX_DETAIL_BYTES - 4)).unwrap().len(),
        MAX_DETAIL_BYTES
    );
    let state = PageState::InProgress { carry: carry() };
    let fits = PageDetails {
        problems: list(MAX_DETAIL_BYTES - 4),
        ..PageDetails::default()
    };
    store
        .save_page_reading(tenant, doc, 1, &state, &fits)
        .await
        .unwrap();
    let too_big = PageDetails {
        problems: list(MAX_DETAIL_BYTES - 3),
        ..PageDetails::default()
    };
    let error = store
        .save_page_reading(tenant, doc, 2, &state, &too_big)
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::InvalidPageReading("problems, findings and notes are lists")
        ),
        "{error:?}"
    );
}

#[sqlx::test]
async fn a_stored_row_that_passes_the_database_but_not_the_rules_is_refused_when_read(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    // The database checks the shape of a state, not that a reading is an object or how long a
    // listed place is; the store does, on the way out as on the way in.
    raw(
        &pool,
        format!(
            "insert into page_readings (tenant_id, document_id, page, state, reading, rung, attempt, model, confidence_basis) \
             values ('{}', '{}', 1, 'read', '[1]', 0, 1, 'm', 'text_confirmed')",
            tenant.0, doc.0
        ),
    )
    .await
    .unwrap();
    let error = store.load_page_readings(tenant, doc).await.unwrap_err();
    assert!(matches!(error, Error::InvalidStoredPageReading(_)), "{error:?}");

    raw(
        &pool,
        format!("delete from page_readings where document_id = '{}'", doc.0),
    )
    .await
    .unwrap();
    raw(
        &pool,
        format!(
            "insert into page_readings (tenant_id, document_id, page, state, carry, unconfirmed) \
             values ('{}', '{}', 1, 'in_progress', '{{}}', array['{}'])",
            tenant.0,
            doc.0,
            "p".repeat(101)
        ),
    )
    .await
    .unwrap();
    let error = store.load_page_readings(tenant, doc).await.unwrap_err();
    assert!(matches!(error, Error::InvalidStoredPageReading(_)), "{error:?}");
}

#[sqlx::test]
async fn json_that_is_small_as_text_but_large_in_the_table_is_a_clear_refusal(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    // 29 000 numbers are about 58 000 bytes of text, but the table keeps each as a number of
    // its own and its limit is on that form.
    let numbers = Value::Array(vec![json!(1); 29_000]);
    assert!(serde_json::to_string(&numbers).unwrap().len() <= MAX_DETAIL_BYTES);
    let details = PageDetails {
        findings: numbers,
        ..PageDetails::default()
    };
    let error = store
        .save_page_reading(
            tenant,
            doc,
            1,
            &PageState::InProgress { carry: carry() },
            &details,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::InvalidPageReading("a reading, a carry or a list is too large for the table")
        ),
        "{error:?}"
    );
    assert_eq!(stored_count(&pool, doc).await, 0);
}
