// Written by Kone Tshivhinda & Claude
//! These tests need a running Postgres: set `DATABASE_URL`, for example
//! `postgres://clearcut@127.0.0.1:54329/clearcut`. Each test gets
//! its own throwaway database with the migrations applied (`sqlx::test`).

use super::*;
use clearcut_core::auth::{Principal, Scope};
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

pub(crate) fn new_document(seed: u8) -> NewDocument {
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
pub(crate) async fn app_store(pool: &PgPool) -> Store {
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
pub(crate) async fn new_tenant(pool: &PgPool, name: &str) -> Result<TenantId> {
    Store::new(pool.clone()).create_tenant(name).await
}

/// A restricted-role store with one tenant and one document.
pub(crate) async fn setup(pool: &PgPool) -> (Store, TenantId, DocId) {
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
pub(crate) fn db_message(result: std::result::Result<sqlx::postgres::PgQueryResult, sqlx::Error>) -> String {
    result
        .expect_err("statement should have been refused")
        .to_string()
}

// ---------- schema ----------

#[sqlx::test]
async fn the_migrations_create_every_table(pool: PgPool) {
    let tables: Vec<String> = sqlx::query_scalar(
        "select table_name::text from information_schema.tables \
         where table_schema = 'public' and table_name in ('tenants','documents','events','audit_log','jobs') \
         order by table_name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(tables, ["audit_log", "documents", "events", "jobs", "tenants"]);
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

fn document_of(seed: u8, byte_size: i64) -> NewDocument {
    NewDocument {
        byte_size,
        ..new_document(seed)
    }
}

#[sqlx::test]
async fn storage_used_sums_only_this_tenants_documents(pool: PgPool) {
    let (store, tenant, _) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Practice").await.unwrap();
    store.add_document(other, &document_of(2, 500)).await.unwrap();
    assert_eq!(store.tenant_storage_used(tenant).await.unwrap(), 1234);
    assert_eq!(store.tenant_storage_used(other).await.unwrap(), 500);
}

#[sqlx::test]
async fn a_document_of_a_given_size_is_found_only_for_its_own_tenant(pool: PgPool) {
    let (store, tenant, _) = setup(&pool).await;
    assert!(store.tenant_has_document_of_size(tenant, 1234).await.unwrap());
    assert!(!store.tenant_has_document_of_size(tenant, 1233).await.unwrap());
    let other = new_tenant(&pool, "Other Practice").await.unwrap();
    assert!(!store.tenant_has_document_of_size(other, 1234).await.unwrap());
}

#[sqlx::test]
async fn a_document_that_would_cross_the_quota_is_refused(pool: PgPool) {
    let (store, tenant, _) = setup(&pool).await;
    let error = store
        .add_document_within(tenant, &document_of(2, 100), 1300)
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::StorageQuotaExceeded {
                used_bytes: 1234,
                quota_bytes: 1300
            }
        ),
        "{error:?}"
    );
    assert_eq!(store.tenant_storage_used(tenant).await.unwrap(), 1234);
}

#[sqlx::test]
async fn a_document_that_exactly_fills_the_quota_is_accepted(pool: PgPool) {
    let (store, tenant, _) = setup(&pool).await;
    let added = store
        .add_document_within(tenant, &document_of(2, 66), 1300)
        .await
        .unwrap();
    assert!(!added.duplicate);
    assert_eq!(store.tenant_storage_used(tenant).await.unwrap(), 1300);
}

#[sqlx::test]
async fn two_uploads_for_one_account_cannot_both_take_the_last_room(pool: PgPool) {
    let (store, tenant, _) = setup(&pool).await;
    let quota = 1300;
    let first = document_of(2, 66);
    let second = document_of(3, 66);
    let (a, b) = tokio::join!(
        store.add_document_within(tenant, &first, quota),
        store.add_document_within(tenant, &second, quota)
    );
    assert_eq!(
        usize::from(a.is_ok()) + usize::from(b.is_ok()),
        1,
        "only one upload fits"
    );
    assert_eq!(store.tenant_storage_used(tenant).await.unwrap(), quota);
}

#[sqlx::test]
async fn a_duplicate_is_accepted_even_when_the_account_is_full(pool: PgPool) {
    let (store, tenant, _) = setup(&pool).await;
    let again = store
        .add_document_within(tenant, &new_document(1), 1234)
        .await
        .unwrap();
    assert!(again.duplicate);
}

#[sqlx::test]
async fn one_tenants_quota_does_not_limit_another(pool: PgPool) {
    let (store, tenant, _) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Practice").await.unwrap();
    store
        .add_document_within(other, &document_of(2, 500), 1234)
        .await
        .unwrap();
    assert_eq!(store.tenant_storage_used(tenant).await.unwrap(), 1234);
    assert_eq!(store.tenant_storage_used(other).await.unwrap(), 500);
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
pub(crate) async fn as_tenant(pool: &PgPool, tenant: TenantId) -> sqlx::Transaction<'static, sqlx::Postgres> {
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
pub(crate) async fn count(pool: &PgPool, sql: &str) -> i64 {
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

// ---------- API keys (ADR-0007) ----------

fn new_key(name: &str, scopes: &[Scope]) -> NewApiKey {
    NewApiKey {
        name: name.to_string(),
        scopes: scopes.to_vec(),
        lifetime_days: None,
    }
}

/// A key made the way the operator's `clearcut-admin` command makes the first one.
async fn operator_key(store: &Store, tenant: TenantId, scopes: &[Scope]) -> CreatedApiKey {
    store
        .create_api_key_as_operator(tenant, &new_key("test key", scopes))
        .await
        .unwrap()
}

/// A principal holding the given scopes, obtained the real way: create a key
/// and authenticate with it.
async fn principal_with(store: &Store, tenant: TenantId, scopes: &[Scope]) -> Principal {
    let key = operator_key(store, tenant, scopes).await;
    store.authenticate(key.token.expose()).await.unwrap()
}

#[sqlx::test]
async fn a_new_key_authenticates_as_its_tenant_with_exactly_its_scopes(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let key = operator_key(&store, tenant, &[Scope::EventsRead, Scope::DocumentsRead]).await;
    let principal = store.authenticate(key.token.expose()).await.unwrap();
    assert_eq!(principal.tenant(), tenant.0);
    assert_eq!(principal.key_id(), key.id);
    assert_eq!(principal.scopes(), vec![Scope::DocumentsRead, Scope::EventsRead]);
    assert!(!principal.has(Scope::KeysAdmin));
}

#[sqlx::test]
async fn a_key_belongs_to_one_tenant_only(pool: PgPool) {
    let two = two_tenants(&pool).await;
    let key_a = operator_key(&two.app, two.a.0, &[Scope::DocumentsRead]).await;
    let key_b = operator_key(&two.app, two.b.0, &[Scope::DocumentsRead]).await;
    assert_eq!(
        two.app.authenticate(key_a.token.expose()).await.unwrap().tenant(),
        two.a.0 .0
    );
    assert_eq!(
        two.app.authenticate(key_b.token.expose()).await.unwrap().tenant(),
        two.b.0 .0
    );
}

#[sqlx::test]
async fn a_key_lasts_365_days_unless_a_shorter_life_is_asked_for(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let default = operator_key(&store, tenant, &[Scope::DocumentsRead]).await;
    let short = store
        .create_api_key_as_operator(
            tenant,
            &NewApiKey {
                lifetime_days: NonZeroU32::new(30),
                ..new_key("short", &[Scope::DocumentsRead])
            },
        )
        .await
        .unwrap();
    for (key, days) in [(default.id, 365), (short.id, 30)] {
        let exact: bool = sqlx::query_scalar(
            "select expires_at - created_at = make_interval(days => $2) from api_keys where id = $1",
        )
        .bind(key)
        .bind(days)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(exact, "{days} days");
    }
    assert_eq!(DEFAULT_KEY_LIFETIME_DAYS, 365);
}

#[sqlx::test]
async fn the_database_refuses_a_key_that_never_expires(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let generated = keys::generate().unwrap();
    let refused = sqlx::query(
        "insert into api_keys (id, tenant_id, name, secret_hash, scopes, expires_at) \
         values ($1, $2, 'x', $3, array['documents:read'], null)",
    )
    .bind(generated.id)
    .bind(tenant.0)
    .bind(&generated.secret_hash[..])
    .execute(&pool)
    .await;
    assert!(db_message(refused).contains("null value"));
    let _ = store;
}

#[sqlx::test]
async fn a_key_needs_a_name_and_at_least_one_scope(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    assert!(matches!(
        store
            .create_api_key_as_operator(tenant, &new_key("   ", &[Scope::DocumentsRead]))
            .await,
        Err(Error::BlankName)
    ));
    assert!(matches!(
        store
            .create_api_key_as_operator(tenant, &new_key("empty", &[]))
            .await,
        Err(Error::NoScopes)
    ));
    assert_eq!(count(&pool, "select count(*) from api_keys").await, 0);
}

#[sqlx::test]
async fn only_a_hash_of_the_secret_is_stored_anywhere(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let key = operator_key(&store, tenant, &[Scope::KeysAdmin]).await;
    let admin = store.authenticate(key.token.expose()).await.unwrap();
    store.revoke_api_key(&admin, key.id).await.unwrap(); // also writes an audit row

    let secret_hex = key.token.expose()[5 + 32 + 1..].to_string();
    let parsed = keys::parse(key.token.expose()).unwrap();
    let stored: Vec<u8> = sqlx::query_scalar("select secret_hash from api_keys where id = $1")
        .bind(key.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, parsed.secret_hash.to_vec());
    // The secret appears in no column of either table.
    for table in ["api_keys", "audit_log"] {
        let hits = count(
            &pool,
            &format!("select count(*) from {table} t where t::text like '%{secret_hex}%'"),
        )
        .await;
        assert_eq!(hits, 0, "{table}");
    }
}

#[sqlx::test]
async fn bad_keys_all_fail_in_exactly_the_same_way(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let admin = principal_with(&store, tenant, &[Scope::KeysAdmin]).await;

    let good = operator_key(&store, tenant, &[Scope::DocumentsRead]).await;
    let parsed = keys::parse(good.token.expose()).unwrap();
    let wrong_secret = format!("ccut_{}_{}", good.id.simple(), "0".repeat(64));
    let unknown = keys::generate().unwrap().token.expose().to_string();
    let revoked = operator_key(&store, tenant, &[Scope::DocumentsRead]).await;
    store.revoke_api_key(&admin, revoked.id).await.unwrap();
    let expired = operator_key(&store, tenant, &[Scope::DocumentsRead]).await;
    sqlx::query(
        "update api_keys set created_at = now() - interval '2 days', \
         expires_at = now() - interval '1 second' where id = $1",
    )
    .bind(expired.id)
    .execute(&pool)
    .await
    .unwrap();
    let _ = parsed;

    // Sanity: the good key works; the others do not, and say nothing different.
    assert!(store.authenticate(good.token.expose()).await.is_ok());
    let attempts = [
        ("wrong secret", wrong_secret),
        ("unknown id", unknown),
        ("revoked", revoked.token.expose().to_string()),
        ("expired", expired.token.expose().to_string()),
        ("malformed", "ccut_nonsense".to_string()),
        ("empty", String::new()),
        ("bearer prefix left on", format!("Bearer {}", good.token.expose())),
    ];
    let mut messages = std::collections::HashSet::new();
    for (what, presented) in attempts {
        let result = store.authenticate(&presented).await;
        assert!(matches!(result, Err(Error::Unauthenticated)), "{what}");
        messages.insert(result.unwrap_err().to_string());
    }
    assert_eq!(messages.len(), 1, "every failure must look identical");
}

#[sqlx::test]
async fn revoking_takes_effect_at_once_and_repeating_it_is_harmless(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let admin = principal_with(&store, tenant, &[Scope::KeysAdmin]).await;
    let key = operator_key(&store, tenant, &[Scope::DocumentsRead]).await;
    assert!(store.authenticate(key.token.expose()).await.is_ok());
    store.revoke_api_key(&admin, key.id).await.unwrap();
    assert!(matches!(
        store.authenticate(key.token.expose()).await,
        Err(Error::Unauthenticated)
    ));
    store.revoke_api_key(&admin, key.id).await.unwrap(); // again: fine
    assert!(matches!(
        store.revoke_api_key(&admin, Uuid::from_u128(5)).await,
        Err(Error::ApiKeyNotFound)
    ));
}

#[sqlx::test]
async fn one_tenant_cannot_revoke_or_list_another_tenants_keys(pool: PgPool) {
    let two = two_tenants(&pool).await;
    let admin_a = principal_with(&two.app, two.a.0, &[Scope::KeysAdmin]).await;
    let key_b = operator_key(&two.app, two.b.0, &[Scope::DocumentsRead]).await;

    assert!(matches!(
        two.app.revoke_api_key(&admin_a, key_b.id).await,
        Err(Error::ApiKeyNotFound)
    ));
    assert!(two.app.authenticate(key_b.token.expose()).await.is_ok());

    let listed = two.app.list_api_keys(&admin_a).await.unwrap();
    assert!(listed.iter().all(|k| k.id != key_b.id));
    assert_eq!(listed.len(), 1); // only A's own admin key
}

#[sqlx::test]
async fn managing_keys_needs_the_keys_admin_scope(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let reader = principal_with(&store, tenant, &[Scope::DocumentsRead, Scope::EventsRead]).await;
    let missing = Error::Auth(clearcut_core::Error::MissingScope(Scope::KeysAdmin));
    let created = store
        .create_api_key(&reader, &new_key("sneaky", &[Scope::KeysAdmin]))
        .await;
    assert!(matches!(&created, Err(e) if e.to_string() == missing.to_string()));
    assert!(matches!(
        store.revoke_api_key(&reader, reader.key_id()).await,
        Err(Error::Auth(_))
    ));
    assert!(matches!(store.list_api_keys(&reader).await, Err(Error::Auth(_))));
    // Nothing was created or revoked.
    assert_eq!(count(&pool, "select count(*) from api_keys").await, 1);
    assert_eq!(
        count(
            &pool,
            "select count(*) from api_keys where revoked_at is not null"
        )
        .await,
        0
    );
}

#[sqlx::test]
async fn an_admin_can_create_list_and_revoke_keys_for_their_own_tenant(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let admin = principal_with(&store, tenant, &[Scope::KeysAdmin, Scope::DocumentsWrite]).await;
    let created = store
        .create_api_key(&admin, &new_key("ci uploader", &[Scope::DocumentsWrite]))
        .await
        .unwrap();
    let uploader = store.authenticate(created.token.expose()).await.unwrap();
    assert_eq!(uploader.tenant(), tenant.0);
    assert_eq!(uploader.scopes(), vec![Scope::DocumentsWrite]);

    let listed = store.list_api_keys(&admin).await.unwrap();
    let info = listed.iter().find(|k| k.id == created.id).unwrap();
    assert_eq!(info.name, "ci uploader");
    assert_eq!(info.scopes, vec![Scope::DocumentsWrite]);
    assert!(info.revoked_at.is_none());
    assert!(info.last_used_at.is_some()); // the authenticate call above

    store.revoke_api_key(&admin, created.id).await.unwrap();
    let after = store.list_api_keys(&admin).await.unwrap();
    assert!(after
        .iter()
        .find(|k| k.id == created.id)
        .unwrap()
        .revoked_at
        .is_some());
}

#[sqlx::test]
async fn last_used_is_recorded_but_at_most_once_a_minute(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let key = operator_key(&store, tenant, &[Scope::DocumentsRead]).await;
    let last_used = || async {
        sqlx::query_scalar::<_, Option<time::OffsetDateTime>>(
            "select last_used_at from api_keys where id = $1",
        )
        .bind(key.id)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert!(last_used().await.is_none());

    store.authenticate(key.token.expose()).await.unwrap();
    let first = last_used().await.unwrap();
    store.authenticate(key.token.expose()).await.unwrap();
    assert_eq!(
        last_used().await.unwrap(),
        first,
        "a second use inside a minute writes nothing"
    );

    sqlx::query("update api_keys set last_used_at = now() - interval '2 minutes' where id = $1")
        .bind(key.id)
        .execute(&pool)
        .await
        .unwrap();
    store.authenticate(key.token.expose()).await.unwrap();
    let refreshed = last_used().await.unwrap();
    assert!(refreshed > time::OffsetDateTime::now_utc() - time::Duration::minutes(1));
}

#[sqlx::test]
async fn the_application_role_cannot_read_secret_hashes_or_tamper_with_keys(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let key = operator_key(&store, tenant, &[Scope::DocumentsRead]).await;

    // Reading the safe columns is fine.
    let mut tx = as_tenant(store.pool(), tenant).await;
    let visible: i64 = sqlx::query_scalar("select count(id) from api_keys")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(visible, 1);
    drop(tx);

    for statement in [
        "select secret_hash from api_keys",
        "select * from api_keys",
        "update api_keys set scopes = array['keys:admin']",
        "update api_keys set expires_at = now() + interval '100 years'",
        "update api_keys set secret_hash = decode(repeat('00', 32), 'hex')",
        "update api_keys set name = 'renamed'",
        "delete from api_keys",
        "truncate api_keys",
    ] {
        let mut tx = as_tenant(store.pool(), tenant).await;
        let refused = sqlx::query(statement).execute(&mut *tx).await;
        assert!(db_message(refused).contains("permission denied"), "{statement}");
    }
    // Nothing changed.
    assert!(store.authenticate(key.token.expose()).await.is_ok());
}

#[sqlx::test]
async fn without_a_tenant_the_application_role_sees_no_keys(pool: PgPool) {
    let two = two_tenants(&pool).await;
    operator_key(&two.app, two.a.0, &[Scope::DocumentsRead]).await;
    operator_key(&two.app, two.b.0, &[Scope::DocumentsRead]).await;
    assert_eq!(count(&pool, "select count(*) from api_keys").await, 2);
    assert_eq!(count(two.app.pool(), "select count(id) from api_keys").await, 0);

    let mut tx = as_tenant(two.app.pool(), two.a.0).await;
    let own: i64 = sqlx::query_scalar("select count(id) from api_keys")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(own, 1);
}

#[sqlx::test]
async fn authentication_is_a_vetted_security_definer_function_for_the_app_role_only(pool: PgPool) {
    let _ = setup(&pool).await;
    let (secdef, owner, config): (bool, String, Option<Vec<String>>) = sqlx::query_as(
        "select p.prosecdef, pg_get_userbyid(p.proowner), p.proconfig \
         from pg_proc p where p.proname = 'authenticate_api_key'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(secdef);
    assert_eq!(owner, "clearcut_authenticator");
    assert!(config
        .unwrap_or_default()
        .iter()
        .any(|c| c.starts_with("search_path=")));

    let (login, superuser, bypass): (bool, bool, bool) = sqlx::query_as(
        "select rolcanlogin, rolsuper, rolbypassrls from pg_roles where rolname = 'clearcut_authenticator'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!login && !superuser && !bypass);

    let probe = format!("probe_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("create role {probe} nologin")))
        .execute(&pool)
        .await
        .unwrap();
    let can_probe: bool = sqlx::query_scalar(
        "select has_function_privilege($1, 'authenticate_api_key(uuid, bytea)', 'execute')",
    )
    .bind(&probe)
    .fetch_one(&pool)
    .await
    .unwrap();
    let can_app: bool = sqlx::query_scalar(
        "select has_function_privilege('clearcut_app', 'authenticate_api_key(uuid, bytea)', 'execute')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("drop role {probe}")))
        .execute(&pool)
        .await
        .unwrap();
    assert!(!can_probe && can_app);
}

#[sqlx::test]
async fn a_scope_this_version_does_not_know_fails_closed(pool: PgPool) {
    // Version skew or corruption: never quietly drop the scope and carry on.
    let (store, tenant, _doc) = setup(&pool).await;
    let key = operator_key(&store, tenant, &[Scope::DocumentsRead]).await;
    sqlx::query("update api_keys set scopes = array['documents:read', 'bogus:scope'] where id = $1")
        .bind(key.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        store.authenticate(key.token.expose()).await,
        Err(Error::InvalidStoredScopes(_))
    ));
}

#[sqlx::test]
async fn creating_and_revoking_are_audited_without_the_secret(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let admin = principal_with(&store, tenant, &[Scope::KeysAdmin, Scope::EventsRead]).await;
    let created = store
        .create_api_key(&admin, &new_key("audited", &[Scope::EventsRead]))
        .await
        .unwrap();
    store.revoke_api_key(&admin, created.id).await.unwrap();

    let rows: Vec<(String, serde_json::Value, serde_json::Value, String)> = sqlx::query_as(
        "select action, actor, detail, subject_id from audit_log \
         where tenant_id = $1 and subject_id = $2 order by id",
    )
    .bind(tenant.0)
    .bind(created.id.to_string())
    .fetch_all(&pool)
    .await
    .unwrap();
    let actions: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(actions, ["api_key.created", "api_key.revoked"]);
    // Who did it: the admin's key.
    assert_eq!(
        rows[0].1,
        json!({ "type": "api_key", "id": admin.key_id().to_string() })
    );
    // What it was, with no secret.
    assert_eq!(rows[0].2["name"], "audited");
    assert_eq!(rows[0].2["scopes"], json!(["events:read"]));
}

#[sqlx::test]
async fn the_created_key_does_not_print_its_secret(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let key = operator_key(&store, tenant, &[Scope::DocumentsRead]).await;
    let secret_hex = key.token.expose()[5 + 32 + 1..].to_string();
    assert!(!format!("{key:?}").contains(&secret_hex));
    assert!(!format!("{:#?}", key).contains(&secret_hex));
}

#[sqlx::test]
async fn a_longer_life_than_the_default_cannot_be_asked_for(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let key = store
        .create_api_key_as_operator(
            tenant,
            &NewApiKey {
                lifetime_days: NonZeroU32::new(100_000),
                ..new_key("greedy", &[Scope::DocumentsRead])
            },
        )
        .await
        .unwrap();
    let exact: bool = sqlx::query_scalar(
        "select expires_at - created_at = make_interval(days => 365) from api_keys where id = $1",
    )
    .bind(key.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(exact, "the life is capped at the default, never longer");
}

#[sqlx::test]
async fn repeated_scopes_are_stored_once_and_the_name_is_trimmed(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let key = store
        .create_api_key_as_operator(
            tenant,
            &new_key(
                "  padded name  ",
                &[Scope::EventsRead, Scope::DocumentsRead, Scope::EventsRead],
            ),
        )
        .await
        .unwrap();
    let (name, scopes): (String, Vec<String>) =
        sqlx::query_as("select name, scopes from api_keys where id = $1")
            .bind(key.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(name, "padded name");
    assert_eq!(scopes, ["documents:read", "events:read"]);
}

#[sqlx::test]
async fn a_key_admin_cannot_hand_out_scopes_it_does_not_hold(pool: PgPool) {
    // keys:admin lets you manage keys. It must not let you mint a key that can
    // read invoices, upload or review: no scope implies another (ADR-0007).
    let (store, tenant, _doc) = setup(&pool).await;
    let admin = principal_with(&store, tenant, &[Scope::KeysAdmin]).await;
    for scope in [
        Scope::DocumentsRead,
        Scope::DocumentsWrite,
        Scope::EventsRead,
        Scope::ReviewWrite,
    ] {
        let result = store.create_api_key(&admin, &new_key("escalate", &[scope])).await;
        assert!(
            matches!(&result, Err(Error::Auth(clearcut_core::Error::MissingScope(missing))) if *missing == scope),
            "{scope:?}: {result:?}"
        );
    }
    assert_eq!(
        count(&pool, "select count(*) from api_keys where name = 'escalate'").await,
        0
    );
}

#[sqlx::test]
async fn one_missing_scope_refuses_the_whole_request(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let admin = principal_with(&store, tenant, &[Scope::KeysAdmin, Scope::DocumentsRead]).await;
    let result = store
        .create_api_key(
            &admin,
            &new_key("mixed", &[Scope::DocumentsRead, Scope::EventsRead]),
        )
        .await;
    assert!(matches!(
        result,
        Err(Error::Auth(clearcut_core::Error::MissingScope(Scope::EventsRead)))
    ));
    assert_eq!(
        count(&pool, "select count(*) from api_keys where name = 'mixed'").await,
        0
    );
    assert_eq!(
        count(
            &pool,
            "select count(*) from audit_log where subject_type = 'api_key' and detail ->> 'name' = 'mixed'"
        )
        .await,
        0
    );
}

#[sqlx::test]
async fn a_key_admin_can_hand_out_any_subset_of_the_scopes_it_holds(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let held = [Scope::KeysAdmin, Scope::DocumentsRead, Scope::EventsRead];
    let admin = principal_with(&store, tenant, &held).await;
    for wanted in [
        vec![Scope::DocumentsRead],
        vec![Scope::EventsRead, Scope::DocumentsRead],
        vec![Scope::KeysAdmin], // a peer administrator, which the admin already is
        held.to_vec(),
    ] {
        let created = store
            .create_api_key(&admin, &new_key("subset", &wanted))
            .await
            .unwrap();
        let principal = store.authenticate(created.token.expose()).await.unwrap();
        let mut expected = wanted.clone();
        expected.sort();
        expected.dedup();
        assert_eq!(principal.scopes(), expected);
    }
}

#[sqlx::test]
async fn the_operator_can_still_issue_every_scope(pool: PgPool) {
    // The one deliberate way to create a key with scopes nobody holds yet: the
    // operator's admin command, e.g. for a tenant's first key.
    let (store, tenant, _doc) = setup(&pool).await;
    let key = operator_key(&store, tenant, &Scope::ALL).await;
    let principal = store.authenticate(key.token.expose()).await.unwrap();
    assert_eq!(principal.scopes(), Scope::ALL.to_vec());
}

#[sqlx::test]
async fn the_operator_can_revoke_a_leaked_key_without_holding_an_admin_key(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let leaked = operator_key(&store, tenant, &[Scope::DocumentsRead]).await;
    assert!(store.authenticate(leaked.token.expose()).await.is_ok());

    store.revoke_api_key_as_operator(tenant, leaked.id).await.unwrap();

    assert!(matches!(
        store.authenticate(leaked.token.expose()).await,
        Err(Error::Unauthenticated)
    ));
    // Audited as the operator's doing, with no secret.
    let (actor, action): (serde_json::Value, String) = sqlx::query_as(
        "select actor, action from audit_log where subject_id = $1 and action = 'api_key.revoked'",
    )
    .bind(leaked.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(actor, json!({ "type": "operator" }));
    assert_eq!(action, "api_key.revoked");
}

#[sqlx::test]
async fn an_operator_revoke_is_safe_to_repeat_and_names_unknown_keys(pool: PgPool) {
    let two = two_tenants(&pool).await;
    let key_b = operator_key(&two.app, two.b.0, &[Scope::DocumentsRead]).await;

    two.app
        .revoke_api_key_as_operator(two.b.0, key_b.id)
        .await
        .unwrap();
    two.app
        .revoke_api_key_as_operator(two.b.0, key_b.id)
        .await
        .unwrap(); // again: fine
    let revocations = count(
        &pool,
        "select count(*) from audit_log where action = 'api_key.revoked'",
    )
    .await;
    assert_eq!(revocations, 1, "a repeat writes no second audit entry");

    // A key id that does not exist for the named tenant is not found.
    assert!(matches!(
        two.app
            .revoke_api_key_as_operator(two.b.0, Uuid::from_u128(9))
            .await,
        Err(Error::ApiKeyNotFound)
    ));
    // Naming the wrong tenant does not revoke another tenant's key.
    let key_a = operator_key(&two.app, two.a.0, &[Scope::DocumentsRead]).await;
    assert!(matches!(
        two.app.revoke_api_key_as_operator(two.b.0, key_a.id).await,
        Err(Error::ApiKeyNotFound)
    ));
    assert!(two.app.authenticate(key_a.token.expose()).await.is_ok());
}

// ---------- refusing to run with the wrong database role ----------

/// A store connected as an extra login role created just for this test.
async fn store_as_new_role(pool: &PgPool, role_sql: &str) -> (Store, String) {
    let role = format!("probe_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "create role {role} login {role_sql}"
    )))
    .execute(pool)
    .await
    .unwrap();
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "grant usage on schema public to {role}; grant select on all tables in schema public to {role}"
    )))
    .execute(pool)
    .await
    .unwrap();
    let options = pool.connect_options().as_ref().clone().username(&role);
    let role_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    (Store::new(role_pool), role)
}

#[sqlx::test]
async fn the_restricted_service_role_passes_the_startup_check(pool: PgPool) {
    let _ = setup(&pool).await;
    app_store(&pool).await.verify_service_role().await.unwrap();
}

#[sqlx::test]
async fn a_superuser_is_refused_because_it_would_bypass_tenant_isolation(pool: PgPool) {
    let _ = setup(&pool).await;
    let result = Store::new(pool.clone()).verify_service_role().await;
    assert!(
        matches!(result, Err(Error::UnsafeServiceRole(reason)) if reason.contains("superuser")),
        "{result:?}"
    );
}

#[sqlx::test]
async fn a_role_that_can_bypass_row_level_security_is_refused(pool: PgPool) {
    let _ = setup(&pool).await;
    let (store, role) = store_as_new_role(&pool, "nosuperuser bypassrls").await;
    let result = store.verify_service_role().await;
    store.pool().close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "drop owned by {role}; drop role {role}"
    )))
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        matches!(result, Err(Error::UnsafeServiceRole(reason)) if reason.contains("row-level")),
        "{result:?}"
    );
}

#[sqlx::test]
async fn a_role_that_owns_the_tenant_tables_is_refused(pool: PgPool) {
    // Table owners are not subject to row-level security.
    let _ = setup(&pool).await;
    let (store, role) = store_as_new_role(&pool, "nosuperuser nobypassrls").await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "alter table documents owner to {role}"
    )))
    .execute(&pool)
    .await
    .unwrap();
    let result = store.verify_service_role().await;
    store.pool().close().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "alter table documents owner to clearcut; drop owned by {role}; drop role {role}"
    )))
    .execute(&pool)
    .await
    .ok();
    assert!(
        matches!(result, Err(Error::UnsafeServiceRole(reason)) if reason.contains("owns")),
        "{result:?}"
    );
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

// ---------- where a document's file is ----------

#[sqlx::test]
async fn a_documents_file_details_are_returned_as_stored(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let file = store.document_file(tenant, doc).await.unwrap();
    let expected = new_document(1);
    assert_eq!(file.storage_key, expected.storage_key);
    assert_eq!(file.content_type, expected.content_type);
    assert_eq!(file.byte_size, 1234);
    assert_eq!(file.sha256, expected.sha256);
}

#[sqlx::test]
async fn another_tenants_or_a_missing_documents_file_is_not_found(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other Advisory").await.unwrap();
    assert!(matches!(
        store.document_file(other, doc).await,
        Err(Error::DocumentNotFound)
    ));
    assert!(matches!(
        store.document_file(tenant, DocId(Uuid::new_v4())).await,
        Err(Error::DocumentNotFound)
    ));
}

// ---------- retention ----------

/// Moves a document into the past. The admin connection is used because the
/// application role may not change a document's timestamps.
async fn age(pool: &PgPool, doc: DocId, days: i32) {
    sqlx::query("update documents set created_at = now() - make_interval(days => $2) where id = $1")
        .bind(doc.0)
        .bind(days)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test]
async fn retention_finds_aged_documents_in_every_account_oldest_first(pool: PgPool) {
    let (store, a, a_doc) = setup(&pool).await;
    let b = new_tenant(&pool, "Other Practice").await.unwrap();
    let b_old = store.add_document(b, &new_document(2)).await.unwrap().id;
    let b_new = store.add_document(b, &new_document(3)).await.unwrap().id;
    age(&pool, a_doc, 400).await;
    age(&pool, b_old, 366).await;

    let candidates = store.retention_candidates(365, 100).await.unwrap();
    assert_eq!(candidates, vec![(a, a_doc), (b, b_old)]);
    assert!(!candidates.iter().any(|(_, doc)| *doc == b_new));
}

#[sqlx::test]
async fn retention_waits_for_a_running_job_but_not_a_queued_one(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    age(&pool, doc, 400).await;
    let actor = json!({ "type": "system" });
    let job = store
        .enqueue_job(
            tenant,
            &NewJob::new(doc, clearcut_core::jobs::JobKind::ProcessDocument),
            &actor,
        )
        .await
        .unwrap();
    // Queued means nothing has started: the document is still due to go, and erasure cancels the
    // job first. This is what lets a document expire on a server with no worker.
    assert_eq!(
        store.retention_candidates(365, 100).await.unwrap(),
        vec![(tenant, doc)]
    );

    sqlx::query(
        "update jobs set state = 'running', started_at = now(), attempts = 1, \
         lease_token = gen_random_uuid(), leased_by = 'test', \
         lease_expires_at = now() + interval '1 minute' where id = $1",
    )
    .bind(job.id)
    .execute(&pool)
    .await
    .unwrap();
    // Running means work in progress: left alone until it ends.
    assert!(store.retention_candidates(365, 100).await.unwrap().is_empty());

    sqlx::query(
        "update jobs set state = 'cancelled', finished_at = now(), \
         lease_token = null, leased_by = null, lease_expires_at = null where id = $1",
    )
    .bind(job.id)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        store.retention_candidates(365, 100).await.unwrap(),
        vec![(tenant, doc)]
    );
}

#[sqlx::test]
async fn retention_keeps_a_document_younger_than_the_period(pool: PgPool) {
    let (store, _, doc) = setup(&pool).await;
    age(&pool, doc, 364).await;
    assert!(store.retention_candidates(365, 100).await.unwrap().is_empty());
}

#[sqlx::test]
async fn retention_returns_at_most_the_batch_size(pool: PgPool) {
    let (store, tenant, first) = setup(&pool).await;
    for seed in 2..5u8 {
        let doc = store.add_document(tenant, &new_document(seed)).await.unwrap().id;
        age(&pool, doc, 400).await;
    }
    age(&pool, first, 401).await;
    assert_eq!(store.retention_candidates(365, 2).await.unwrap().len(), 2);
}

#[sqlx::test]
async fn retention_refuses_a_period_below_one_day(pool: PgPool) {
    let (store, _, _) = setup(&pool).await;
    assert!(store.retention_candidates(0, 100).await.is_err());
}

#[sqlx::test]
async fn expired_documents_is_a_vetted_security_definer_function_owned_by_the_retention_role(pool: PgPool) {
    let _ = setup(&pool).await;
    let (secdef, owner, config): (bool, String, Option<Vec<String>>) = sqlx::query_as(
        "select p.prosecdef, pg_get_userbyid(p.proowner), p.proconfig \
         from pg_proc p where p.proname = 'expired_documents'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(secdef);
    assert_eq!(owner, "clearcut_retention");
    assert!(config
        .unwrap_or_default()
        .iter()
        .any(|c| c.starts_with("search_path=")));
}

#[sqlx::test]
async fn only_the_application_role_may_call_expired_documents(pool: PgPool) {
    let _ = setup(&pool).await;
    let probe = format!("probe_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(sqlx::AssertSqlSafe(format!("create role {probe} nologin")))
        .execute(&pool)
        .await
        .unwrap();
    let can_probe: bool = sqlx::query_scalar(
        "select has_function_privilege($1, 'expired_documents(integer, integer)', 'execute')",
    )
    .bind(&probe)
    .fetch_one(&pool)
    .await
    .unwrap();
    let can_app: bool = sqlx::query_scalar(
        "select has_function_privilege('clearcut_app', 'expired_documents(integer, integer)', 'execute')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("drop role {probe}")))
        .execute(&pool)
        .await
        .unwrap();
    assert!(!can_probe, "an arbitrary role must not list aged documents");
    assert!(can_app);
}
