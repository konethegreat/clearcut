// Written by Kone Tshivhinda & Claude
//! How each page of a document will be read (ADR-0017): one row per page, replaced as a
//! whole each time the analysis stage runs.
//!
//! The rules that make the decision live in `clearcut-core` (`page_read`); this module keeps
//! what the core produced and hands it back, re-checked by the core's own rules as it is
//! read. A row holds facts about a page and none of its text.

use crate::{require_document, DocId, Error, Result, Store, TenantId};
use clearcut_core::page_read::{PageRead, StoredPageRead};

/// The most pages one document may have rows for. The worker refuses far fewer; this only
/// keeps one call from writing an absurd number of rows.
pub const MAX_PAGES: usize = 10_000;

/// The rows must be exactly pages 1 to n, once each.
fn check_pages(reads: &[PageRead]) -> Result<()> {
    if reads.is_empty() {
        return Err(Error::InvalidPageReads("a document has at least one page"));
    }
    if reads.len() > MAX_PAGES {
        return Err(Error::InvalidPageReads("a document has too many pages"));
    }
    let mut pages: Vec<u32> = reads.iter().map(PageRead::page).collect();
    pages.sort_unstable();
    let in_order = pages.iter().zip(1u32..).all(|(page, expected)| *page == expected);
    if !in_order {
        return Err(Error::InvalidPageReads("the pages are not exactly 1 to n"));
    }
    Ok(())
}

impl Store {
    /// Stores how every page of `document` will be read, replacing any earlier rows, and
    /// sets the document's page count to match, all in one transaction: a failure leaves
    /// the earlier rows as they were. `reads` must be exactly pages 1 to n (in any order).
    pub async fn replace_page_reads(
        &self,
        tenant: TenantId,
        document: DocId,
        reads: &[PageRead],
    ) -> Result<()> {
        check_pages(reads)?;
        let page_count = i32::try_from(reads.len()).map_err(|_| Error::InvalidPageReads("too many pages"))?;
        let mut tx = self.begin(tenant).await?;
        require_document(&mut *tx, tenant, document).await?;
        sqlx::query("delete from page_reads where tenant_id = $1 and document_id = $2")
            .bind(tenant.0)
            .bind(document.0)
            .execute(&mut *tx)
            .await?;
        for read in reads {
            let reasons: Vec<&str> = read.reasons().iter().map(|reason| reason.as_str()).collect();
            // Page numbers and agreements are small (checked above and by the core).
            let page = i32::try_from(read.page()).map_err(|_| Error::InvalidPageReads("too many pages"))?;
            let agreement = read
                .ocr_agreement()
                .map(i16::try_from)
                .transpose()
                .map_err(|_| Error::InvalidPageReads("an agreement is out of range"))?;
            sqlx::query(
                "insert into page_reads \
                 (tenant_id, document_id, page, text_source, verdict, reasons, ocr_agreement, route, analysed_with) \
                 values ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(tenant.0)
            .bind(document.0)
            .bind(page)
            .bind(read.text_source().as_str())
            .bind(read.verdict().as_str())
            .bind(&reasons)
            .bind(agreement)
            .bind(read.route().as_str())
            .bind(read.analysed_with())
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "update documents set page_count = $3, updated_at = now() where tenant_id = $1 and id = $2",
        )
        .bind(tenant.0)
        .bind(document.0)
        .bind(page_count)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// How every page of `document` will be read, in page order; empty if the analysis has
    /// not run. Each row is re-checked by the core's rules as it is read.
    pub async fn load_page_reads(&self, tenant: TenantId, document: DocId) -> Result<Vec<PageRead>> {
        let mut tx = self.begin(tenant).await?;
        require_document(&mut *tx, tenant, document).await?;
        type Row = (i32, String, String, Vec<String>, Option<i16>, String, String);
        let rows: Vec<Row> = sqlx::query_as(
            "select page, text_source, verdict, reasons, ocr_agreement, route, analysed_with \
             from page_reads where tenant_id = $1 and document_id = $2 order by page",
        )
        .bind(tenant.0)
        .bind(document.0)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter()
            .map(
                |(page, text_source, verdict, reasons, agreement, route, analysed_with)| {
                    let invalid = |why: &str| Error::InvalidStoredPageRead(why.to_string());
                    let page = u32::try_from(page).map_err(|_| invalid("a negative page number"))?;
                    let agreement = agreement
                        .map(u16::try_from)
                        .transpose()
                        .map_err(|_| invalid("a negative agreement"))?;
                    let reasons: Vec<&str> = reasons.iter().map(String::as_str).collect();
                    PageRead::from_stored(StoredPageRead {
                        page,
                        text_source: &text_source,
                        verdict: &verdict,
                        reasons: &reasons,
                        ocr_agreement: agreement,
                        route: &route,
                        analysed_with: &analysed_with,
                    })
                    .map_err(|e| invalid(&e.to_string()))
                },
            )
            .collect()
    }
}

#[cfg(test)]
mod tests;
