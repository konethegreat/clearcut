// Written by Kone Tshivhinda & Claude
//! These tests need a running Postgres: set `DATABASE_URL`, for example
//! `postgres://clearcut@127.0.0.1:54329/clearcut`. Each test gets
//! its own throwaway database with the migrations applied (`sqlx::test`).

use super::*;
use clearcut_core::event::{
    view, Citation, Confidence, DocumentId as CoreDocumentId, Event, EventId, EventParts, EventPayload,
    Extractor,
};
use clearcut_core::extraction::{derive_events, DueTerms, ExtractedLine, Extraction, Party, Sourced};
use clearcut_core::{Currency, InvoiceTotals, Money, TaxCategory};
use serde_json::json;
use sqlx::PgPool;
use std::num::NonZeroU32;
use time::macros::date as d;

// ---------- builders ----------

fn zar(minor: i64) -> Money {
    Money::zar(minor)
}

fn at<T>(value: T) -> Sourced<T> {
    Sourced {
        value,
        page: NonZeroU32::new(1).unwrap(),
        locator: None,
        confidence: Confidence::new(950).unwrap(),
    }
}

/// A correct R18 250.00 invoice for the stored document `doc`.
fn extraction(doc: DocId) -> Extraction {
    let line = |description: &str, qty, price, total, tax| {
        at(ExtractedLine {
            description: description.into(),
            quantity_milli: qty,
            unit_price: zar(price),
            line_total: zar(total),
            tax,
        })
    };
    let party = |name: &str, vat: &str| {
        at(Party {
            name: name.into(),
            address: Some("1 Main Rd, Rosebank".into()),
            vat_number: Some(vat.into()),
        })
    };
    Extraction {
        document: doc.as_core_id(),
        extractor: Extractor::System,
        currency: Currency::Zar,
        invoice_number: Some(at("INV-000001".to_string())),
        invoice_date: Some(at(d!(2026 - 03 - 31))),
        supplier: Some(party("Acme (Pty) Ltd", "4123456789")),
        recipient: Some(party("Client Co", "4987654321")),
        lines: vec![
            line("Consulting", 10_000, 150_000, 1_500_000, TaxCategory::Standard),
            line("Export licence", 1_000, 100_000, 100_000, TaxCategory::ZeroRated),
        ],
        stated_totals: Some(at(InvoiceTotals {
            net: zar(1_600_000),
            vat: zar(225_000),
            gross: zar(1_825_000),
        })),
        due: Some(at(DueTerms {
            due: d!(2026 - 04 - 30),
            terms: Some("30 days".into()),
        })),
        credit_note: None,
    }
}

fn new_document(seed: u8) -> NewDocument {
    NewDocument {
        original_filename: format!("invoice-{seed}.pdf"),
        content_type: "application/pdf".into(),
        byte_size: 1234,
        sha256: [seed; 32],
        storage_key: format!("tenants/t/docs/{seed}"),
        page_count: Some(2),
    }
}

/// A store that connects as the restricted application role `clearcut_app`
/// (ADR-0007), the way the running service does. `pool` (from `sqlx::test`)
/// is a superuser connection to the same throwaway database: tests use it as
/// the "admin" to set things up and to look at the database from outside.
async fn app_store(pool: &PgPool) -> Store {
    let options = pool.connect_options().as_ref().clone().username("clearcut_app");
    let app_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    Store::new(app_pool)
}

/// Creating a tenant is an owner-level operation (the `clearcut-admin`
/// command), so it goes through the admin connection.
async fn new_tenant(pool: &PgPool, name: &str) -> Result<TenantId> {
    Store::new(pool.clone()).create_tenant(name).await
}

/// A restricted-role store with one tenant and one document.
async fn setup(pool: &PgPool) -> (Store, TenantId, DocId) {
    let tenant = new_tenant(pool, "Acme Advisory").await.unwrap();
    let store = app_store(pool).await;
    let doc = store.add_document(tenant, &new_document(1)).await.unwrap().id;
    (store, tenant, doc)
}

fn events_of(doc: DocId, run: u32, earlier: &[Event]) -> Vec<Event> {
    derive_events(&extraction(doc), NonZeroU32::new(run).unwrap(), earlier)
        .unwrap()
        .events
}

fn ids(events: &[Event]) -> Vec<String> {
    events.iter().map(|e| e.id().as_str().to_string()).collect()
}

/// The text of the database error behind a failed raw statement.
fn db_message(result: std::result::Result<sqlx::postgres::PgQueryResult, sqlx::Error>) -> String {
    result
        .expect_err("statement should have been refused")
        .to_string()
}

// ---------- schema ----------

#[sqlx::test]
async fn the_migrations_create_every_table(pool: PgPool) {
    let tables: Vec<String> = sqlx::query_scalar(
        "select table_name::text from information_schema.tables \
         where table_schema = 'public' and table_name in ('tenants','documents','events','audit_log') \
         order by table_name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(tables, ["audit_log", "documents", "events", "tenants"]);
}

// ---------- tenants and documents ----------

#[sqlx::test]
async fn a_new_document_starts_as_received_and_can_move_on(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    assert_eq!(
        store.document_status(tenant, doc).await.unwrap(),
        DocumentStatus::Received
    );
    for status in [
        DocumentStatus::Processing,
        DocumentStatus::NeedsReview,
        DocumentStatus::Done,
    ] {
        store.set_document_status(tenant, doc, status).await.unwrap();
        assert_eq!(store.document_status(tenant, doc).await.unwrap(), status);
    }
}

#[sqlx::test]
async fn uploading_the_same_content_twice_returns_the_same_document(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let again = store.add_document(tenant, &new_document(1)).await.unwrap();
    assert_eq!(again.id, doc);
    assert!(again.duplicate);
    let different = store.add_document(tenant, &new_document(2)).await.unwrap();
    assert_ne!(different.id, doc);
    assert!(!different.duplicate);
}

#[sqlx::test]
async fn another_tenant_uploading_the_same_content_gets_its_own_document(pool: PgPool) {
    let (store, _tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Practice").await.unwrap();
    let theirs = store.add_document(other, &new_document(1)).await.unwrap();
    assert_ne!(theirs.id, doc);
    assert!(!theirs.duplicate);
}

#[sqlx::test]
async fn failed_is_not_a_document_status(pool: PgPool) {
    let (_store, _tenant, doc) = setup(&pool).await;
    let refused = sqlx::query("update documents set status = 'failed' where id = $1")
        .bind(doc.0)
        .execute(&pool)
        .await;
    assert!(db_message(refused).contains("documents_status_check"));
}

#[sqlx::test]
async fn a_document_is_invisible_to_another_tenant(pool: PgPool) {
    let (store, _tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Practice").await.unwrap();
    assert!(matches!(
        store.document_status(other, doc).await,
        Err(Error::DocumentNotFound)
    ));
    assert!(matches!(
        store.set_document_status(other, doc, DocumentStatus::Done).await,
        Err(Error::DocumentNotFound)
    ));
}

// ---------- events: round trip ----------

#[sqlx::test]
async fn events_derived_by_the_core_come_back_identical_and_in_order(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let events = events_of(doc, 1, &[]);
    assert!(events.len() > 5);
    store.append_events(tenant, doc, &events).await.unwrap();
    let loaded = store.load_events(tenant, doc).await.unwrap();
    assert_eq!(loaded, events);
}

#[sqlx::test]
async fn appending_the_same_events_again_changes_nothing(pool: PgPool) {
    // A retried job must be safe: same IDs, same content.
    let (store, tenant, doc) = setup(&pool).await;
    let events = events_of(doc, 1, &[]);
    store.append_events(tenant, doc, &events).await.unwrap();
    store.append_events(tenant, doc, &events).await.unwrap();
    assert_eq!(store.load_events(tenant, doc).await.unwrap(), events);
}

#[sqlx::test]
async fn reusing_an_event_id_with_different_content_is_refused(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let events = events_of(doc, 1, &[]);
    store.append_events(tenant, doc, &events).await.unwrap();

    let original = &events[0];
    let altered = Event::new(EventParts {
        id: original.id().clone(),
        date: original.date(),
        payload: EventPayload::SupplierIdentified {
            name: "Somebody Else".into(),
            vat_number: None,
        },
        citation: original.citation().clone(),
        extractor: original.extractor().clone(),
        confidence: original.confidence(),
        supersedes: None,
    })
    .unwrap();
    let result = store.append_events(tenant, doc, &[altered]).await;
    assert!(matches!(result, Err(Error::EventIdConflict(id)) if id == original.id().as_str()));
    assert_eq!(store.load_events(tenant, doc).await.unwrap(), events);
}

#[sqlx::test]
async fn a_batch_with_one_bad_event_stores_nothing(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let run1 = events_of(doc, 1, &[]);
    store.append_events(tenant, doc, &run1[..1]).await.unwrap();

    let altered = Event::new(EventParts {
        id: run1[0].id().clone(),
        date: None,
        payload: EventPayload::SupplierIdentified {
            name: "Somebody Else".into(),
            vat_number: None,
        },
        citation: run1[0].citation().clone(),
        extractor: Extractor::System,
        confidence: Confidence::new(1).unwrap(),
        supersedes: None,
    })
    .unwrap();
    // A fresh event first, then the conflicting one: the fresh one must roll back.
    let batch = vec![run1[1].clone(), altered];
    assert!(store.append_events(tenant, doc, &batch).await.is_err());
    assert_eq!(store.load_events(tenant, doc).await.unwrap(), run1[..1].to_vec());
}

#[sqlx::test]
async fn an_event_citing_a_different_document_is_refused(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other_doc = store.add_document(tenant, &new_document(2)).await.unwrap().id;
    let events = events_of(other_doc, 1, &[]); // cites other_doc
    let result = store.append_events(tenant, doc, &events).await;
    assert!(matches!(result, Err(Error::EventForOtherDocument(_))));
    assert!(store.load_events(tenant, doc).await.unwrap().is_empty());
}

// ---------- re-extraction ----------

#[sqlx::test]
async fn a_second_run_is_stored_alongside_the_first_and_supersedes_it(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let run1 = events_of(doc, 1, &[]);
    store.append_events(tenant, doc, &run1).await.unwrap();
    let run2 = events_of(doc, 2, &run1);
    store.append_events(tenant, doc, &run2).await.unwrap();

    let all = store.load_events(tenant, doc).await.unwrap();
    assert_eq!(all.len(), run1.len() + run2.len());
    let current = view::current(&all);
    assert_eq!(
        ids(&current.iter().map(|e| (*e).clone()).collect::<Vec<_>>()),
        ids(&run2)
    );

    // The generated column mirrors `supersedes` in the stored JSON.
    let superseded: i64 =
        sqlx::query_scalar("select count(*) from events where document_id = $1 and supersedes is not null")
            .bind(doc.0)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(superseded as usize, run2.len());
}

// ---------- generated columns ----------

#[sqlx::test]
async fn the_event_type_column_is_generated_from_the_stored_json(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .append_events(tenant, doc, &events_of(doc, 1, &[]))
        .await
        .unwrap();
    let types: Vec<String> = sqlx::query_scalar(
        "select distinct event_type from events where document_id = $1 order by event_type",
    )
    .bind(doc.0)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        types,
        [
            "due_date_stated",
            "line_item_billed",
            "supplier_identified",
            "totals_stated",
            "vat_calculated"
        ]
    );
}

// ---------- append-only ----------

#[sqlx::test]
async fn events_cannot_be_updated_or_deleted(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .append_events(tenant, doc, &events_of(doc, 1, &[]))
        .await
        .unwrap();
    let update = sqlx::query("update events set recorded_at = now()")
        .execute(&pool)
        .await;
    assert!(db_message(update).contains("append-only"));
    let delete = sqlx::query("delete from events").execute(&pool).await;
    assert!(db_message(delete).contains("append-only"));
}

#[sqlx::test]
async fn the_old_erasure_setting_no_longer_opens_anything(pool: PgPool) {
    // ADR-0007 retired `clearcut.erasure`: any session could set it, so it
    // protected nothing. Even a superuser setting it gets nowhere now; only
    // the erase_document function (running as clearcut_eraser) may delete.
    let (store, tenant, doc) = setup(&pool).await;
    let events = events_of(doc, 1, &[]);
    store.append_events(tenant, doc, &events).await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("set local clearcut.erasure = 'on'")
        .execute(&mut *tx)
        .await
        .unwrap();
    let delete = sqlx::query("delete from events").execute(&mut *tx).await;
    assert!(db_message(delete).contains("append-only"));
    drop(tx);
    assert_eq!(store.load_events(tenant, doc).await.unwrap().len(), events.len());
}

#[sqlx::test]
async fn the_audit_log_is_append_only_too(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    store
        .record_audit(
            tenant,
            &AuditEntry {
                actor: json!({ "type": "system" }),
                action: "document.uploaded".into(),
                subject_type: "document".into(),
                subject_id: "abc".into(),
                detail: json!({ "pages": 2 }),
            },
        )
        .await
        .unwrap();
    let (action, detail): (String, serde_json::Value) =
        sqlx::query_as("select action, detail from audit_log where tenant_id = $1")
            .bind(tenant.0)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(action, "document.uploaded");
    assert_eq!(detail, json!({ "pages": 2 }));

    let update = sqlx::query("update audit_log set action = 'x'")
        .execute(&pool)
        .await;
    assert!(db_message(update).contains("append-only"));
    let delete = sqlx::query("delete from audit_log").execute(&pool).await;
    assert!(db_message(delete).contains("append-only"));
}

#[sqlx::test]
async fn the_audit_log_cannot_be_deleted_by_anyone_with_or_without_the_old_setting(pool: PgPool) {
    // The audit log has no erasure path at all (ADR-0006 amendment, ADR-0007).
    let (store, tenant, _doc) = setup(&pool).await;
    store
        .record_audit(
            tenant,
            &AuditEntry {
                actor: json!({ "type": "system" }),
                action: "document.uploaded".into(),
                subject_type: "document".into(),
                subject_id: "abc".into(),
                detail: json!({}),
            },
        )
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("set local clearcut.erasure = 'on'")
        .execute(&mut *tx)
        .await
        .unwrap();
    let delete = sqlx::query("delete from audit_log").execute(&mut *tx).await;
    assert!(db_message(delete).contains("append-only"));
    drop(tx);

    let remaining: i64 = sqlx::query_scalar("select count(*) from audit_log")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 1);
}

// ---------- tenant isolation ----------

#[sqlx::test]
async fn another_tenant_can_neither_read_nor_write_a_documents_events(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let events = events_of(doc, 1, &[]);
    store.append_events(tenant, doc, &events).await.unwrap();

    let intruder = new_tenant(&pool, "Intruder").await.unwrap();
    assert!(matches!(
        store.load_events(intruder, doc).await,
        Err(Error::DocumentNotFound)
    ));
    assert!(matches!(
        store.append_events(intruder, doc, &events).await,
        Err(Error::DocumentNotFound)
    ));
}

#[sqlx::test]
async fn the_database_itself_refuses_an_event_under_another_tenants_document(pool: PgPool) {
    // Bypassing the store: the composite foreign key still holds.
    let (_store, _tenant, doc) = setup(&pool).await;
    let intruder = new_tenant(&pool, "Intruder").await.unwrap();
    let event = events_of(doc, 1, &[]).remove(0);
    let refused =
        sqlx::query("insert into events (tenant_id, document_id, id, body) values ($1, $2, $3, $4)")
            .bind(intruder.0)
            .bind(doc.0)
            .bind(event.id().as_str())
            .bind(serde_json::to_value(&event).unwrap())
            .execute(&pool)
            .await;
    assert!(db_message(refused).contains("foreign key"));
}

// ---------- database checks on the stored JSON ----------

#[sqlx::test]
async fn a_body_whose_id_differs_from_its_row_id_is_refused(pool: PgPool) {
    let (_store, tenant, doc) = setup(&pool).await;
    let event = events_of(doc, 1, &[]).remove(0);
    let refused =
        sqlx::query("insert into events (tenant_id, document_id, id, body) values ($1, $2, 'other-id', $3)")
            .bind(tenant.0)
            .bind(doc.0)
            .bind(serde_json::to_value(&event).unwrap())
            .execute(&pool)
            .await;
    assert!(db_message(refused).contains("check"));
}

#[sqlx::test]
async fn a_body_citing_another_document_is_refused(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other_doc = store.add_document(tenant, &new_document(2)).await.unwrap().id;
    let event = events_of(other_doc, 1, &[]).remove(0);
    let refused =
        sqlx::query("insert into events (tenant_id, document_id, id, body) values ($1, $2, $3, $4)")
            .bind(tenant.0)
            .bind(doc.0)
            .bind(event.id().as_str())
            .bind(serde_json::to_value(&event).unwrap())
            .execute(&pool)
            .await;
    assert!(db_message(refused).contains("check"));
}

// ---------- reading is re-validated ----------

#[sqlx::test]
async fn an_invalid_stored_event_is_an_error_when_read_not_silently_accepted(pool: PgPool) {
    // Something wrote straight to the table: the row passes the database
    // checks, but the event inside breaks a core rule (confidence above 1000).
    let (store, tenant, doc) = setup(&pool).await;
    let event = events_of(doc, 1, &[]).remove(0);
    let mut body = serde_json::to_value(&event).unwrap();
    body["confidence"] = json!(1001);
    sqlx::query("insert into events (tenant_id, document_id, id, body) values ($1, $2, $3, $4)")
        .bind(tenant.0)
        .bind(doc.0)
        .bind(event.id().as_str())
        .bind(body)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        store.load_events(tenant, doc).await,
        Err(Error::InvalidStoredEvent(_))
    ));
}

// ---------- the restricted role and row-level security (ADR-0007) ----------

/// A transaction on `pool` that has said which tenant it acts for, exactly as
/// every `Store` operation does.
async fn as_tenant(pool: &PgPool, tenant: TenantId) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("select set_config('clearcut.tenant', $1, true)")
        .bind(tenant.0.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    tx
}

/// Runs a counting query. The SQL is built in these tests from table names
/// and UUIDs we generated ourselves, never from outside input, which is what
/// `AssertSqlSafe` states.
async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(pool)
        .await
        .unwrap()
}

struct Two {
    app: Store,
    a: (TenantId, DocId),
    b: (TenantId, DocId),
}

/// Two tenants, each with one document and its events.
async fn two_tenants(pool: &PgPool) -> Two {
    let ta = new_tenant(pool, "Tenant A").await.unwrap();
    let tb = new_tenant(pool, "Tenant B").await.unwrap();
    let app = app_store(pool).await;
    let da = app.add_document(ta, &new_document(1)).await.unwrap().id;
    let db = app.add_document(tb, &new_document(2)).await.unwrap().id;
    app.append_events(ta, da, &events_of(da, 1, &[])).await.unwrap();
    app.append_events(tb, db, &events_of(db, 1, &[])).await.unwrap();
    Two {
        app,
        a: (ta, da),
        b: (tb, db),
    }
}

#[sqlx::test]
async fn the_application_role_is_a_plain_restricted_role(pool: PgPool) {
    let _ = setup(&pool).await; // runs the migrations' role setup
    let (superuser, bypass, createrole, createdb, login): (bool, bool, bool, bool, bool) = sqlx::query_as(
        "select rolsuper, rolbypassrls, rolcreaterole, rolcreatedb, rolcanlogin \
             from pg_roles where rolname = 'clearcut_app'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!superuser && !bypass && !createrole && !createdb);
    assert!(login);
    // It must not own the tables (owners escape row-level security).
    let owned = count(
        &pool,
        "select count(*) from pg_tables where schemaname = 'public' and tableowner = 'clearcut_app'",
    )
    .await;
    assert_eq!(owned, 0);
}

#[sqlx::test]
async fn the_eraser_role_cannot_log_in(pool: PgPool) {
    let _ = setup(&pool).await;
    let (login, superuser, bypass): (bool, bool, bool) = sqlx::query_as(
        "select rolcanlogin, rolsuper, rolbypassrls from pg_roles where rolname = 'clearcut_eraser'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!login && !superuser && !bypass);
}

#[sqlx::test]
async fn with_no_tenant_set_the_application_role_sees_nothing(pool: PgPool) {
    let two = two_tenants(&pool).await;
    for table in ["tenants", "documents", "events"] {
        let admin_sees = count(&pool, &format!("select count(*) from {table}")).await;
        let app_sees = count(two.app.pool(), &format!("select count(*) from {table}")).await;
        assert!(
            admin_sees > 0,
            "{table} should have rows for the test to mean anything"
        );
        assert_eq!(app_sees, 0, "{table} must fail closed");
    }
}

#[sqlx::test]
async fn a_query_that_forgets_the_tenant_filter_still_only_sees_its_own_rows(pool: PgPool) {
    let two = two_tenants(&pool).await;
    let all_events = count(&pool, "select count(*) from events").await;
    let mut tx = as_tenant(two.app.pool(), two.a.0).await;
    // Deliberately sloppy: no `where tenant_id = ...` anywhere.
    let events: i64 = sqlx::query_scalar("select count(*) from events")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let tenants_in_events: i64 = sqlx::query_scalar("select count(distinct tenant_id) from events")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let documents: i64 = sqlx::query_scalar("select count(*) from documents")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let tenants: i64 = sqlx::query_scalar("select count(*) from tenants")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(events > 0 && events < all_events);
    assert_eq!((tenants_in_events, documents, tenants), (1, 1, 1));
}

#[sqlx::test]
async fn the_tenant_setting_does_not_outlive_its_transaction(pool: PgPool) {
    let two = two_tenants(&pool).await;
    let tx = as_tenant(two.app.pool(), two.a.0).await;
    tx.commit().await.unwrap();
    // Whatever connection serves these queries, no tenant is set on it.
    for _ in 0..8 {
        let setting: Option<String> = sqlx::query_scalar("select current_setting('clearcut.tenant', true)")
            .fetch_one(two.app.pool())
            .await
            .unwrap();
        assert!(setting.unwrap_or_default().is_empty());
        assert_eq!(count(two.app.pool(), "select count(*) from documents").await, 0);
    }
}

#[sqlx::test]
async fn the_application_role_cannot_write_rows_for_another_tenant(pool: PgPool) {
    let two = two_tenants(&pool).await;
    let mut tx = as_tenant(two.app.pool(), two.a.0).await;
    let refused = sqlx::query(
        "insert into documents (tenant_id, original_filename, content_type, byte_size, sha256, storage_key) \
         values ($1, 'x.pdf', 'application/pdf', 1, $2, 'k')",
    )
    .bind(two.b.0 .0)
    .bind(&[9u8; 32][..])
    .execute(&mut *tx)
    .await;
    assert!(db_message(refused).contains("row-level security"));
}

#[sqlx::test]
async fn the_application_role_cannot_change_or_remove_events_or_audit_rows(pool: PgPool) {
    let two = two_tenants(&pool).await;
    two.app
        .record_audit(
            two.a.0,
            &AuditEntry {
                actor: json!({ "type": "system" }),
                action: "document.uploaded".into(),
                subject_type: "document".into(),
                subject_id: "x".into(),
                detail: json!({}),
            },
        )
        .await
        .unwrap();
    for statement in [
        "update events set recorded_at = now()",
        "delete from events",
        "truncate events",
        "update audit_log set action = 'x'",
        "delete from audit_log",
        "truncate audit_log",
        "delete from documents",
        "update tenants set name = 'x'",
    ] {
        let mut tx = as_tenant(two.app.pool(), two.a.0).await;
        let refused = sqlx::query(statement).execute(&mut *tx).await;
        assert!(db_message(refused).contains("permission denied"), "{statement}");
    }
    assert!(count(&pool, "select count(*) from events").await > 0);
}

#[sqlx::test]
async fn the_application_role_cannot_create_tenants(pool: PgPool) {
    let two = two_tenants(&pool).await;
    assert!(matches!(
        two.app.create_tenant("Sneaky").await,
        Err(Error::Database(_))
    ));
    assert_eq!(count(&pool, "select count(*) from tenants").await, 2);
}

// ---------- erase_document ----------

#[sqlx::test]
async fn erasing_a_document_removes_its_events_and_row_and_leaves_an_audit_entry(pool: PgPool) {
    let two = two_tenants(&pool).await;
    let b_events_before = count(
        &pool,
        &format!("select count(*) from events where document_id = '{}'", two.b.1 .0),
    )
    .await;

    two.app.erase_document(two.a.0, two.a.1).await.unwrap();

    let a_events = count(
        &pool,
        &format!("select count(*) from events where document_id = '{}'", two.a.1 .0),
    )
    .await;
    let a_docs = count(
        &pool,
        &format!("select count(*) from documents where id = '{}'", two.a.1 .0),
    )
    .await;
    assert_eq!((a_events, a_docs), (0, 0));

    // The other tenant is untouched.
    let b_events = count(
        &pool,
        &format!("select count(*) from events where document_id = '{}'", two.b.1 .0),
    )
    .await;
    assert_eq!(b_events, b_events_before);
    assert!(two.app.load_events(two.b.0, two.b.1).await.is_ok());

    // One audit entry: what happened and to what, and no invoice content.
    let (action, subject, detail, actor): (String, String, serde_json::Value, serde_json::Value) =
        sqlx::query_as(
            "select action, subject_id, detail, actor from audit_log \
             where tenant_id = $1 and action = 'document.erased'",
        )
        .bind(two.a.0 .0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(action, "document.erased");
    assert_eq!(subject, two.a.1 .0.to_string());
    assert_eq!(detail, json!({}));
    assert_eq!(actor, json!({ "type": "system" }));
}

#[sqlx::test]
async fn erasing_another_tenants_document_is_refused_and_changes_nothing(pool: PgPool) {
    let two = two_tenants(&pool).await;
    let events_before = count(&pool, "select count(*) from events").await;
    let result = two.app.erase_document(two.a.0, two.b.1).await;
    assert!(matches!(result, Err(Error::DocumentNotFound)));
    assert_eq!(count(&pool, "select count(*) from events").await, events_before);
    assert_eq!(count(&pool, "select count(*) from documents").await, 2);
    assert_eq!(
        count(
            &pool,
            "select count(*) from audit_log where action = 'document.erased'"
        )
        .await,
        0
    );
}

#[sqlx::test]
async fn erasing_a_document_that_does_not_exist_is_not_found(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let missing = DocId(uuid::Uuid::from_u128(42));
    assert!(matches!(
        store.erase_document(tenant, missing).await,
        Err(Error::DocumentNotFound)
    ));
}

#[sqlx::test]
async fn the_erase_function_refuses_to_act_for_a_tenant_that_is_not_the_current_one(pool: PgPool) {
    // Called directly, bypassing the Store: the function itself checks.
    let two = two_tenants(&pool).await;
    let mut tx = as_tenant(two.app.pool(), two.a.0).await;
    let refused = sqlx::query("select erase_document($1, $2)")
        .bind(two.b.0 .0)
        .bind(two.b.1 .0)
        .execute(&mut *tx)
        .await;
    assert!(db_message(refused).contains("tenant"));
    drop(tx);
    assert_eq!(count(&pool, "select count(*) from documents").await, 2);

    // And with no tenant set at all it refuses too.
    let refused = sqlx::query("select erase_document($1, $2)")
        .bind(two.a.0 .0)
        .bind(two.a.1 .0)
        .execute(two.app.pool())
        .await;
    assert!(db_message(refused).contains("tenant"));
    assert_eq!(count(&pool, "select count(*) from documents").await, 2);
}

#[sqlx::test]
async fn erase_document_is_a_vetted_security_definer_function_owned_by_the_eraser(pool: PgPool) {
    let _ = setup(&pool).await;
    let (secdef, owner, config): (bool, String, Option<Vec<String>>) = sqlx::query_as(
        "select p.prosecdef, pg_get_userbyid(p.proowner), p.proconfig \
         from pg_proc p where p.proname = 'erase_document'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(secdef);
    assert_eq!(owner, "clearcut_eraser");
    // A fixed search_path, so a caller cannot redirect the function's lookups.
    assert!(config
        .unwrap_or_default()
        .iter()
        .any(|c| c.starts_with("search_path=")));
}

#[sqlx::test]
async fn only_the_application_role_may_call_erase_document(pool: PgPool) {
    let _ = setup(&pool).await;
    let probe = format!("probe_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("create role {probe} nologin")))
        .execute(&pool)
        .await
        .unwrap();
    let can_probe: bool =
        sqlx::query_scalar("select has_function_privilege($1, 'erase_document(uuid, uuid)', 'execute')")
            .bind(&probe)
            .fetch_one(&pool)
            .await
            .unwrap();
    let can_app: bool = sqlx::query_scalar(
        "select has_function_privilege('clearcut_app', 'erase_document(uuid, uuid)', 'execute')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("drop role {probe}")))
        .execute(&pool)
        .await
        .unwrap();
    assert!(!can_probe, "an arbitrary role must not be able to erase");
    assert!(can_app);
}

// ---------- ids ----------

#[test]
fn a_stored_document_id_is_the_core_id_a_citation_uses() {
    let id = DocId(uuid::Uuid::nil());
    assert_eq!(
        id.as_core_id(),
        CoreDocumentId::new("00000000-0000-0000-0000-000000000000")
    );
    let _ = (
        Citation {
            document: id.as_core_id(),
            page: NonZeroU32::MIN,
            locator: None,
        },
        EventId::new("x"),
    );
}
