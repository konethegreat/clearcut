// Written by Kone Tshivhinda & Claude
//! These tests need a running Postgres: set `DATABASE_URL`, for example
//! `postgres://clearcut:clearcut_dev@127.0.0.1:54329/clearcut`. Each test gets
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

/// A store with one tenant and one document.
async fn setup(pool: PgPool) -> (Store, TenantId, DocId) {
    let store = Store::new(pool);
    let tenant = store.create_tenant("Acme Advisory").await.unwrap();
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
    let (store, tenant, doc) = setup(pool).await;
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
    let (store, tenant, doc) = setup(pool).await;
    let again = store.add_document(tenant, &new_document(1)).await.unwrap();
    assert_eq!(again.id, doc);
    assert!(again.duplicate);
    let different = store.add_document(tenant, &new_document(2)).await.unwrap();
    assert_ne!(different.id, doc);
    assert!(!different.duplicate);
}

#[sqlx::test]
async fn another_tenant_uploading_the_same_content_gets_its_own_document(pool: PgPool) {
    let (store, _tenant, doc) = setup(pool).await;
    let other = store.create_tenant("Other Practice").await.unwrap();
    let theirs = store.add_document(other, &new_document(1)).await.unwrap();
    assert_ne!(theirs.id, doc);
    assert!(!theirs.duplicate);
}

#[sqlx::test]
async fn failed_is_not_a_document_status(pool: PgPool) {
    let (store, _tenant, doc) = setup(pool).await;
    let refused = sqlx::query("update documents set status = 'failed' where id = $1")
        .bind(doc.0)
        .execute(store.pool())
        .await;
    assert!(db_message(refused).contains("documents_status_check"));
}

#[sqlx::test]
async fn a_document_is_invisible_to_another_tenant(pool: PgPool) {
    let (store, _tenant, doc) = setup(pool).await;
    let other = store.create_tenant("Other Practice").await.unwrap();
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
    let (store, tenant, doc) = setup(pool).await;
    let events = events_of(doc, 1, &[]);
    assert!(events.len() > 5);
    store.append_events(tenant, doc, &events).await.unwrap();
    let loaded = store.load_events(tenant, doc).await.unwrap();
    assert_eq!(loaded, events);
}

#[sqlx::test]
async fn appending_the_same_events_again_changes_nothing(pool: PgPool) {
    // A retried job must be safe: same IDs, same content.
    let (store, tenant, doc) = setup(pool).await;
    let events = events_of(doc, 1, &[]);
    store.append_events(tenant, doc, &events).await.unwrap();
    store.append_events(tenant, doc, &events).await.unwrap();
    assert_eq!(store.load_events(tenant, doc).await.unwrap(), events);
}

#[sqlx::test]
async fn reusing_an_event_id_with_different_content_is_refused(pool: PgPool) {
    let (store, tenant, doc) = setup(pool).await;
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
    let (store, tenant, doc) = setup(pool).await;
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
    let (store, tenant, doc) = setup(pool).await;
    let other_doc = store.add_document(tenant, &new_document(2)).await.unwrap().id;
    let events = events_of(other_doc, 1, &[]); // cites other_doc
    let result = store.append_events(tenant, doc, &events).await;
    assert!(matches!(result, Err(Error::EventForOtherDocument(_))));
    assert!(store.load_events(tenant, doc).await.unwrap().is_empty());
}

// ---------- re-extraction ----------

#[sqlx::test]
async fn a_second_run_is_stored_alongside_the_first_and_supersedes_it(pool: PgPool) {
    let (store, tenant, doc) = setup(pool).await;
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
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(superseded as usize, run2.len());
}

// ---------- generated columns ----------

#[sqlx::test]
async fn the_event_type_column_is_generated_from_the_stored_json(pool: PgPool) {
    let (store, tenant, doc) = setup(pool).await;
    store
        .append_events(tenant, doc, &events_of(doc, 1, &[]))
        .await
        .unwrap();
    let types: Vec<String> = sqlx::query_scalar(
        "select distinct event_type from events where document_id = $1 order by event_type",
    )
    .bind(doc.0)
    .fetch_all(store.pool())
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
    let (store, tenant, doc) = setup(pool).await;
    store
        .append_events(tenant, doc, &events_of(doc, 1, &[]))
        .await
        .unwrap();
    let update = sqlx::query("update events set recorded_at = now()")
        .execute(store.pool())
        .await;
    assert!(db_message(update).contains("append-only"));
    let delete = sqlx::query("delete from events").execute(store.pool()).await;
    assert!(db_message(delete).contains("append-only"));
}

#[sqlx::test]
async fn erasure_can_delete_events_only_inside_an_explicit_transaction_setting(pool: PgPool) {
    let (store, tenant, doc) = setup(pool).await;
    store
        .append_events(tenant, doc, &events_of(doc, 1, &[]))
        .await
        .unwrap();

    let mut tx = store.pool().begin().await.unwrap();
    sqlx::query("set local clearcut.erasure = 'on'")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("delete from events where document_id = $1")
        .bind(doc.0)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(store.load_events(tenant, doc).await.unwrap().is_empty());

    // The setting was local to that transaction: it does not leak.
    let again = sqlx::query("delete from events").execute(store.pool()).await;
    assert!(again.is_ok()); // nothing left to delete, so no row trigger fired
}

#[sqlx::test]
async fn the_audit_log_is_append_only_too(pool: PgPool) {
    let (store, tenant, _doc) = setup(pool).await;
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
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(action, "document.uploaded");
    assert_eq!(detail, json!({ "pages": 2 }));

    let update = sqlx::query("update audit_log set action = 'x'")
        .execute(store.pool())
        .await;
    assert!(db_message(update).contains("append-only"));
    let delete = sqlx::query("delete from audit_log").execute(store.pool()).await;
    assert!(db_message(delete).contains("append-only"));
}

// ---------- tenant isolation ----------

#[sqlx::test]
async fn another_tenant_can_neither_read_nor_write_a_documents_events(pool: PgPool) {
    let (store, tenant, doc) = setup(pool).await;
    let events = events_of(doc, 1, &[]);
    store.append_events(tenant, doc, &events).await.unwrap();

    let intruder = store.create_tenant("Intruder").await.unwrap();
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
    let (store, _tenant, doc) = setup(pool).await;
    let intruder = store.create_tenant("Intruder").await.unwrap();
    let event = events_of(doc, 1, &[]).remove(0);
    let refused =
        sqlx::query("insert into events (tenant_id, document_id, id, body) values ($1, $2, $3, $4)")
            .bind(intruder.0)
            .bind(doc.0)
            .bind(event.id().as_str())
            .bind(serde_json::to_value(&event).unwrap())
            .execute(store.pool())
            .await;
    assert!(db_message(refused).contains("foreign key"));
}

// ---------- database checks on the stored JSON ----------

#[sqlx::test]
async fn a_body_whose_id_differs_from_its_row_id_is_refused(pool: PgPool) {
    let (store, tenant, doc) = setup(pool).await;
    let event = events_of(doc, 1, &[]).remove(0);
    let refused =
        sqlx::query("insert into events (tenant_id, document_id, id, body) values ($1, $2, 'other-id', $3)")
            .bind(tenant.0)
            .bind(doc.0)
            .bind(serde_json::to_value(&event).unwrap())
            .execute(store.pool())
            .await;
    assert!(db_message(refused).contains("check"));
}

#[sqlx::test]
async fn a_body_citing_another_document_is_refused(pool: PgPool) {
    let (store, tenant, doc) = setup(pool).await;
    let other_doc = store.add_document(tenant, &new_document(2)).await.unwrap().id;
    let event = events_of(other_doc, 1, &[]).remove(0);
    let refused =
        sqlx::query("insert into events (tenant_id, document_id, id, body) values ($1, $2, $3, $4)")
            .bind(tenant.0)
            .bind(doc.0)
            .bind(event.id().as_str())
            .bind(serde_json::to_value(&event).unwrap())
            .execute(store.pool())
            .await;
    assert!(db_message(refused).contains("check"));
}

// ---------- reading is re-validated ----------

#[sqlx::test]
async fn an_invalid_stored_event_is_an_error_when_read_not_silently_accepted(pool: PgPool) {
    // Something wrote straight to the table: the row passes the database
    // checks, but the event inside breaks a core rule (confidence above 1000).
    let (store, tenant, doc) = setup(pool).await;
    let event = events_of(doc, 1, &[]).remove(0);
    let mut body = serde_json::to_value(&event).unwrap();
    body["confidence"] = json!(1001);
    sqlx::query("insert into events (tenant_id, document_id, id, body) values ($1, $2, $3, $4)")
        .bind(tenant.0)
        .bind(doc.0)
        .bind(event.id().as_str())
        .bind(body)
        .execute(store.pool())
        .await
        .unwrap();
    assert!(matches!(
        store.load_events(tenant, doc).await,
        Err(Error::InvalidStoredEvent(_))
    ));
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
