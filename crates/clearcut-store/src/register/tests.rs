// Written by Kone Tshivhinda & Claude
//! Store tests for the register selection. They need Postgres, like the rest.

use super::*;
use crate::page_readings::{PageDetails, PageState};
use crate::tests::{new_document, new_tenant, setup};
use crate::DocId;
use clearcut_core::document::DocumentStatus;
use clearcut_core::event::{Citation, Confidence, Event, EventId, EventParts, EventPayload, Extractor};
use sqlx::PgPool;
use std::num::NonZeroU32;
use time::macros::date as d;

fn dated(doc: DocId, id: &str, date: Option<time::Date>, number: &str, supersedes: Option<&Event>) -> Event {
    Event::new(EventParts {
        id: EventId::new(format!("{}:{id}", doc.0)),
        date,
        payload: EventPayload::InvoiceNumberStated {
            number: number.into(),
        },
        citation: Citation {
            document: doc.as_core_id(),
            page: NonZeroU32::MIN,
            locator: None,
        },
        extractor: Extractor::System,
        confidence: Confidence::CERTAIN,
        supersedes: supersedes.map(|earlier| earlier.id().clone()),
    })
    .unwrap()
}

async fn second_document(store: &Store, tenant: TenantId, seed: u8) -> DocId {
    store.add_document(tenant, &new_document(seed)).await.unwrap().id
}

fn period(from: time::Date, to: time::Date) -> Selection {
    Selection::Period { from, to }
}

#[sqlx::test]
async fn a_period_takes_the_documents_dated_in_it(pool: PgPool) {
    let (store, tenant, march) = setup(&pool).await;
    let april = second_document(&store, tenant, 2).await;
    store
        .append_events(
            tenant,
            march,
            &[dated(march, "n", Some(d!(2026 - 03 - 31)), "INV-1", None)],
        )
        .await
        .unwrap();
    store
        .append_events(
            tenant,
            april,
            &[dated(april, "n", Some(d!(2026 - 04 - 15)), "INV-2", None)],
        )
        .await
        .unwrap();
    let input = store
        .register_sources(tenant, &period(d!(2026 - 04 - 01), d!(2026 - 04 - 30)), false)
        .await
        .unwrap();
    assert_eq!(input.facts.len(), 1);
    assert_eq!(input.facts[0].id, april.0.to_string());
    assert_eq!(input.facts[0].events.len(), 1);
    assert_eq!(input.undated_in_tenant, 0);
    assert!(!input.only_ready);
}

#[sqlx::test]
async fn a_period_leaves_undated_documents_out_and_counts_them(pool: PgPool) {
    let (store, tenant, dated_document) = setup(&pool).await;
    let undated = second_document(&store, tenant, 2).await;
    // No events at all: still a document of the tenant with no invoice date.
    second_document(&store, tenant, 3).await;
    store
        .append_events(
            tenant,
            dated_document,
            &[dated(
                dated_document,
                "n",
                Some(d!(2026 - 03 - 31)),
                "INV-1",
                None,
            )],
        )
        .await
        .unwrap();
    store
        .append_events(tenant, undated, &[dated(undated, "n", None, "INV-2", None)])
        .await
        .unwrap();
    let input = store
        .register_sources(tenant, &period(d!(2026 - 03 - 01), d!(2026 - 03 - 31)), false)
        .await
        .unwrap();
    assert_eq!(input.facts.len(), 1);
    assert_eq!(input.undated_in_tenant, 2);
}

#[sqlx::test]
async fn an_undated_document_of_another_tenant_is_not_counted(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .append_events(
            tenant,
            doc,
            &[dated(doc, "n", Some(d!(2026 - 03 - 31)), "INV-1", None)],
        )
        .await
        .unwrap();
    let other = new_tenant(&pool, "Other").await.unwrap();
    let other_doc = store.add_document(other, &new_document(2)).await.unwrap().id;
    store
        .append_events(other, other_doc, &[dated(other_doc, "n", None, "INV-2", None)])
        .await
        .unwrap();
    let input = store
        .register_sources(tenant, &period(d!(2026 - 03 - 01), d!(2026 - 03 - 31)), false)
        .await
        .unwrap();
    assert_eq!(input.facts.len(), 1);
    assert_eq!(input.undated_in_tenant, 0);
}

#[sqlx::test]
async fn a_period_takes_the_date_of_the_latest_current_correction(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    let first = dated(doc, "number-v1", Some(d!(2026 - 03 - 10)), "INV-1", None);
    let correction = dated(doc, "number-v2", Some(d!(2026 - 04 - 20)), "INV-1", Some(&first));
    store
        .append_events(tenant, doc, &[first, correction])
        .await
        .unwrap();
    let march = store
        .register_sources(tenant, &period(d!(2026 - 03 - 01), d!(2026 - 03 - 31)), false)
        .await
        .unwrap();
    assert!(march.facts.is_empty());
    let april = store
        .register_sources(tenant, &period(d!(2026 - 04 - 01), d!(2026 - 04 - 30)), false)
        .await
        .unwrap();
    assert_eq!(april.facts.len(), 1);
    assert_eq!(april.facts[0].events.len(), 1);
    assert_eq!(
        april.facts[0].events[0].id().as_str(),
        format!("{}:number-v2", doc.0)
    );
}

#[sqlx::test]
async fn a_named_selection_takes_documents_whatever_their_date(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .append_events(tenant, doc, &[dated(doc, "n", None, "INV-9", None)])
        .await
        .unwrap();
    let input = store
        .register_sources(
            tenant,
            &Selection::Documents {
                ids: vec![doc.0.to_string()],
            },
            false,
        )
        .await
        .unwrap();
    assert_eq!(input.facts.len(), 1);
    assert_eq!(input.facts[0].status, DocumentStatus::Received);
    assert_eq!(input.facts[0].events.len(), 1);
}

#[sqlx::test]
async fn a_named_document_of_another_tenant_or_a_missing_id_is_not_returned(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let other = new_tenant(&pool, "Other").await.unwrap();
    let other_doc = store.add_document(other, &new_document(2)).await.unwrap().id;
    let missing = Uuid::new_v4();
    let input = store
        .register_sources(
            tenant,
            &Selection::Documents {
                ids: vec![other_doc.0.to_string(), missing.to_string()],
            },
            false,
        )
        .await
        .unwrap();
    assert!(input.facts.is_empty());
}

#[sqlx::test]
async fn the_pages_waiting_for_a_person_are_carried(pool: PgPool) {
    let (store, tenant, doc) = setup(&pool).await;
    store
        .save_page_reading(
            tenant,
            doc,
            2,
            &PageState::NeedsReview {
                reason: ReviewReason::AmountsIncludeVat,
                draft: None,
            },
            &PageDetails::default(),
        )
        .await
        .unwrap();
    let input = store
        .register_sources(
            tenant,
            &Selection::Documents {
                ids: vec![doc.0.to_string()],
            },
            true,
        )
        .await
        .unwrap();
    assert!(input.only_ready);
    assert_eq!(
        input.facts[0].pages,
        vec![PageReview {
            page: 2,
            reason: ReviewReason::AmountsIncludeVat,
        }]
    );
}

#[sqlx::test]
async fn a_document_id_that_is_not_a_uuid_is_refused(pool: PgPool) {
    let (store, tenant, _doc) = setup(&pool).await;
    let error = store
        .register_sources(
            tenant,
            &Selection::Documents {
                ids: vec!["not-a-uuid".into()],
            },
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, Error::InvalidSelectionId(id) if id == "not-a-uuid"));
}
