// Written by Kone Tshivhinda & Claude
//! Which documents a register is made from: a period's documents or a named list, with their
//! current events and the pages that stopped for a person.
//!
//! The register itself is built in `clearcut_core::register`; this module only selects and hands
//! over. A document belongs to a month by the date of its most recent current event that carries
//! one, which is the date the register row shows.

use crate::{parse_status, Error, Result, Store, TenantId};
use clearcut_core::escalation::ReviewReason;
use clearcut_core::event::Event;
use clearcut_core::register::{PageReview, RegisterFacts, RegisterInput, Selection, MAX_DOCUMENTS};
use std::collections::HashMap;
use time::OffsetDateTime;
use uuid::Uuid;

type DocumentRow = (Uuid, String, String, Option<i32>, OffsetDateTime);
type EventRow = (Uuid, serde_json::Value);
type PageRow = (Uuid, i32, String);

impl Store {
    /// The documents a selection names, with their current events and the pages waiting for a
    /// person, in the shape the core's `build` takes.
    ///
    /// A period selection takes a document by the date of its most recent current event that
    /// carries one. A named selection takes the documents whatever their date; an id that is not
    /// a UUID is refused, and an id of another tenant is simply not found.
    pub async fn register_sources(
        &self,
        tenant: TenantId,
        selection: &Selection,
        only_ready: bool,
    ) -> Result<RegisterInput> {
        let mut tx = self.begin(tenant).await?;
        let limit = MAX_DOCUMENTS as i64 + 1;

        let documents: Vec<DocumentRow> = match selection {
            Selection::Period { from, to } => {
                sqlx::query_as(
                    "with current_events as ( \
                     select e.document_id, e.seq, e.event_date \
                     from events e \
                     where e.tenant_id = $1 \
                       and not exists (select 1 from events s \
                                       where s.tenant_id = $1 and s.document_id = e.document_id \
                                         and s.supersedes = e.id) \
                 ), \
                 latest as ( \
                     select distinct on (document_id) document_id, event_date \
                     from current_events \
                     where event_date is not null \
                     order by document_id, seq desc \
                 ) \
                 select d.id, d.original_filename, d.status, d.page_count, d.created_at \
                 from documents d \
                 join latest l on l.document_id = d.id \
                 where d.tenant_id = $1 and l.event_date between $2 and $3 \
                 order by l.event_date, d.created_at, d.id \
                 limit $4",
                )
                .bind(tenant.0)
                .bind(from.to_string())
                .bind(to.to_string())
                .bind(limit)
                .fetch_all(&mut *tx)
                .await?
            }
            Selection::Documents { ids } => {
                let mut wanted = Vec::with_capacity(ids.len());
                for id in ids {
                    let parsed = Uuid::parse_str(id).map_err(|_| Error::InvalidSelectionId(id.clone()))?;
                    wanted.push(parsed);
                }
                sqlx::query_as(
                    "select id, original_filename, status, page_count, created_at \
                     from documents \
                     where tenant_id = $1 and id = any($2) \
                     order by created_at, id \
                     limit $3",
                )
                .bind(tenant.0)
                .bind(&wanted)
                .bind(limit)
                .fetch_all(&mut *tx)
                .await?
            }
        };

        // Every document the tenant has with no invoice date, including one that has no events
        // at all: a period can never select it, so it must not disappear without a count.
        let undated: i64 = sqlx::query_scalar(
            "select count(*) \
             from documents d \
             where d.tenant_id = $1 \
               and not exists ( \
                   select 1 \
                   from events e \
                   where e.tenant_id = $1 \
                     and e.document_id = d.id \
                     and e.event_date is not null \
                     and not exists ( \
                         select 1 from events s \
                         where s.tenant_id = $1 and s.document_id = e.document_id \
                           and s.supersedes = e.id \
                     ) \
               )",
        )
        .bind(tenant.0)
        .fetch_one(&mut *tx)
        .await?;

        let document_ids: Vec<Uuid> = documents.iter().map(|row| row.0).collect();

        let bodies: Vec<EventRow> = sqlx::query_as(
            "select e.document_id, e.body \
             from events e \
             where e.tenant_id = $1 and e.document_id = any($2) \
               and not exists (select 1 from events s \
                               where s.tenant_id = $1 and s.document_id = e.document_id \
                                 and s.supersedes = e.id) \
             order by e.document_id, e.seq",
        )
        .bind(tenant.0)
        .bind(&document_ids)
        .fetch_all(&mut *tx)
        .await?;

        let pages: Vec<PageRow> = sqlx::query_as(
            "select document_id, page, review_reason \
             from page_readings \
             where tenant_id = $1 and document_id = any($2) and state = 'needs_review' \
             order by document_id, page",
        )
        .bind(tenant.0)
        .bind(&document_ids)
        .fetch_all(&mut *tx)
        .await?;

        tx.commit().await?;

        let mut facts: Vec<RegisterFacts> = Vec::with_capacity(documents.len());
        let mut place: HashMap<Uuid, usize> = HashMap::new();
        for (id, filename, status, page_count, created_at) in documents {
            place.insert(id, facts.len());
            facts.push(RegisterFacts {
                id: id.to_string(),
                filename,
                uploaded_at: created_at,
                status: parse_status(&status)?,
                page_count: page_count.and_then(|pages| u32::try_from(pages).ok()),
                pages: Vec::new(),
                events: Vec::new(),
            });
        }
        for (document, body) in bodies {
            let event = serde_json::from_value::<Event>(body)
                .map_err(|error| Error::InvalidStoredEvent(error.to_string()))?;
            if let Some(index) = place.get(&document) {
                facts[*index].events.push(event);
            }
        }
        for (document, page, reason) in pages {
            let reason = ReviewReason::parse(&reason)
                .ok_or_else(|| Error::InvalidStoredPageReading("an unknown review reason".to_string()))?;
            let page = u32::try_from(page)
                .map_err(|_| Error::InvalidStoredPageReading("a negative page number".to_string()))?;
            if let Some(index) = place.get(&document) {
                facts[*index].pages.push(PageReview { page, reason });
            }
        }

        Ok(RegisterInput {
            selection: selection.clone(),
            only_ready,
            undated_in_tenant: usize::try_from(undated).unwrap_or(usize::MAX),
            facts,
        })
    }
}

#[cfg(test)]
mod tests;
