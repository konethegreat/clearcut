// Written by Kone Tshivhinda & Claude
//! What the model router keeps for each page (ADR-0018 section 8): while a page is being read,
//! its **carry** (the router's state machine and the readings kept so far, so a stopped job
//! resumes with the very same call); when it has ended, its final reading, or the best draft of
//! a page that went to a person.
//!
//! **This holds invoice content** and is erased with the document. The store keeps the carry
//! and the readings as the JSON the router produced and does not interpret them: the rules
//! about what a reading is live in `clearcut-core` and `clearcut-router`. What it does check
//! is that a row is one of the three shapes the database allows (so a mistake fails here with a
//! clear message, and the database's own checks stay a second line), and that the sizes are
//! bounded.
//!
//! **A page that has ended is final.** Saving over it is refused ([`Error::PageReadingFinished`]),
//! so a stale writer (a job that was taken over) cannot put a carry back over a result; a
//! re-run deletes the document's rows first.

use crate::{require_document, DocId, Error, Result, Store, TenantId};
use clearcut_core::escalation::{ConfidenceBasis, ReviewReason};
use serde_json::Value;

/// The most a stored reading or carry may hold, as JSON text. The table's own limit is on the
/// stored (binary) form of the JSON, which is larger than its text, so this leaves room.
pub const MAX_JSON_BYTES: usize = 250_000;
/// The most each of a page's problems, findings and notes may hold, as JSON text (the table's
/// limit is 65 536 on the stored form).
pub const MAX_DETAIL_BYTES: usize = 60_000;
/// The most places listed as unconfirmed.
pub const MAX_UNCONFIRMED: usize = 500;
/// The most kinds of instruction-like text listed.
pub const MAX_INJECTION_KINDS: usize = 8;
/// The longest a listed place or kind may be.
pub const MAX_NAME_CHARS: usize = 100;
/// The rungs of the ladder are numbered 0 to this.
pub const MAX_RUNG: u8 = 7;
/// The most attempts a page can have.
pub const MAX_ATTEMPT: u32 = 50;
/// The longest a model name may be.
pub const MAX_MODEL_CHARS: usize = 100;

/// The reading that counts for a page, or its best draft, with where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct KeptReading {
    /// The rung of the ladder (a position in the policy's rungs).
    pub rung: u8,
    /// The attempt that gave it (1 to 50).
    pub attempt: u32,
    pub confidence: ConfidenceBasis,
    /// The model, as the provider named it.
    pub model: String,
    /// The reading as the model gave it (`PageReading` as JSON). **Invoice content.**
    pub reading: Value,
}

/// How far a page has got.
#[derive(Debug, Clone, PartialEq)]
pub enum PageState {
    /// Still being read. The carry is the router's own stored state (opaque here).
    InProgress { carry: Value },
    /// Read and accepted.
    Read(KeptReading),
    /// Handed to a person, with the best draft if any attempt could be converted.
    NeedsReview {
        reason: ReviewReason,
        draft: Option<KeptReading>,
    },
}

/// What went with the reading: fixed names and places, and the checks (which carry amounts).
#[derive(Debug, Clone, PartialEq)]
pub struct PageDetails {
    /// What could not be read exactly and was left out (a JSON list).
    pub problems: Value,
    /// The checks that failed (a JSON list). **Carries amounts.**
    pub findings: Value,
    /// What was recorded and did not block (a JSON list).
    pub notes: Value,
    /// The places of the figures the page's own text did not confirm.
    pub unconfirmed: Vec<String>,
    /// Kinds of instruction-like text found on the page (never the text).
    pub injection_suspected: Vec<String>,
}

impl Default for PageDetails {
    fn default() -> Self {
        PageDetails {
            problems: Value::Array(Vec::new()),
            findings: Value::Array(Vec::new()),
            notes: Value::Array(Vec::new()),
            unconfirmed: Vec::new(),
            injection_suspected: Vec::new(),
        }
    }
}

/// One stored page.
#[derive(Debug, Clone, PartialEq)]
pub struct PageReading {
    pub page: u32,
    pub state: PageState,
    pub details: PageDetails,
}

fn json_size(value: &Value) -> usize {
    serde_json::to_string(value).map_or(usize::MAX, |text| text.len())
}

fn check_object(value: &Value, too_large: &'static str, not_object: &'static str) -> Result<()> {
    if !value.is_object() {
        return Err(Error::InvalidPageReading(not_object));
    }
    if json_size(value) > MAX_JSON_BYTES {
        return Err(Error::InvalidPageReading(too_large));
    }
    Ok(())
}

fn check_kept(
    kept: &KeptReading,
    confidence_ok: impl Fn(ConfidenceBasis) -> bool,
    confidence_why: &'static str,
) -> Result<()> {
    if kept.rung > MAX_RUNG {
        return Err(Error::InvalidPageReading("the rung is outside the ladder"));
    }
    if !(1..=MAX_ATTEMPT).contains(&kept.attempt) {
        return Err(Error::InvalidPageReading("the attempt is outside 1 to 50"));
    }
    if kept.model.is_empty() || kept.model.chars().count() > MAX_MODEL_CHARS {
        return Err(Error::InvalidPageReading("the model name is empty or too long"));
    }
    if !confidence_ok(kept.confidence) {
        return Err(Error::InvalidPageReading(confidence_why));
    }
    check_object(
        &kept.reading,
        "a reading or a carry is too large",
        "a reading is a JSON object",
    )
}

fn check_state(state: &PageState) -> Result<()> {
    match state {
        PageState::InProgress { carry } => check_object(
            carry,
            "a reading or a carry is too large",
            "a carry is a JSON object",
        ),
        PageState::Read(kept) => check_kept(
            kept,
            |basis| basis != ConfidenceBasis::Unresolved,
            "a page that was read has evidence",
        ),
        PageState::NeedsReview { draft, .. } => draft.as_ref().map_or(Ok(()), |kept| {
            check_kept(
                kept,
                |basis| basis == ConfidenceBasis::Unresolved,
                "a draft is unresolved",
            )
        }),
    }
}

fn check_list(value: &Value) -> bool {
    value.is_array() && json_size(value) <= MAX_DETAIL_BYTES
}

fn check_names(names: &[String], most: usize) -> Result<()> {
    if names.len() > most {
        return Err(Error::InvalidPageReading("too many places or kinds are listed"));
    }
    if names
        .iter()
        .any(|name| name.is_empty() || name.chars().count() > MAX_NAME_CHARS)
    {
        return Err(Error::InvalidPageReading(
            "a place or a kind is empty or too long",
        ));
    }
    Ok(())
}

fn check_details(details: &PageDetails) -> Result<()> {
    if ![&details.problems, &details.findings, &details.notes]
        .into_iter()
        .all(check_list)
    {
        return Err(Error::InvalidPageReading(
            "problems, findings and notes are lists",
        ));
    }
    check_names(&details.unconfirmed, MAX_UNCONFIRMED)?;
    check_names(&details.injection_suspected, MAX_INJECTION_KINDS)
}

/// The columns of a state, in the order the table has them.
struct Columns<'a> {
    state: &'static str,
    carry: Option<&'a Value>,
    review_reason: Option<&'static str>,
    kept: Option<&'a KeptReading>,
}

fn columns(state: &PageState) -> Columns<'_> {
    match state {
        PageState::InProgress { carry } => Columns {
            state: "in_progress",
            carry: Some(carry),
            review_reason: None,
            kept: None,
        },
        PageState::Read(kept) => Columns {
            state: "read",
            carry: None,
            review_reason: None,
            kept: Some(kept),
        },
        PageState::NeedsReview { reason, draft } => Columns {
            state: "needs_review",
            carry: None,
            review_reason: Some(reason.as_str()),
            kept: draft.as_ref(),
        },
    }
}

type Row = (
    i32,
    String,
    Option<Value>,
    Option<String>,
    Option<i16>,
    Option<i32>,
    Option<String>,
    Option<String>,
    Option<Value>,
    Value,
    Value,
    Value,
    Vec<String>,
    Vec<String>,
);

fn from_row(row: Row) -> Result<PageReading> {
    let invalid = |why: &str| Error::InvalidStoredPageReading(why.to_string());
    let (
        page,
        state,
        carry,
        reason,
        rung,
        attempt,
        basis,
        model,
        reading,
        problems,
        findings,
        notes,
        unconfirmed,
        injection,
    ) = row;
    let page = u32::try_from(page).map_err(|_| invalid("a negative page number"))?;
    let kept = match (rung, attempt, basis, model, reading) {
        (None, None, None, None, None) => None,
        (Some(rung), Some(attempt), Some(basis), Some(model), Some(reading)) => Some(KeptReading {
            rung: u8::try_from(rung).map_err(|_| invalid("a rung outside the ladder"))?,
            attempt: u32::try_from(attempt).map_err(|_| invalid("a negative attempt"))?,
            confidence: ConfidenceBasis::parse(&basis).ok_or_else(|| invalid("an unknown evidence name"))?,
            model,
            reading,
        }),
        _ => return Err(invalid("a reading with some of its columns missing")),
    };
    let state = match (state.as_str(), carry, reason, kept) {
        ("in_progress", Some(carry), None, None) => PageState::InProgress { carry },
        ("read", None, None, Some(kept)) => PageState::Read(kept),
        ("needs_review", None, Some(reason), draft) => PageState::NeedsReview {
            reason: ReviewReason::parse(&reason).ok_or_else(|| invalid("an unknown review reason"))?,
            draft,
        },
        _ => return Err(invalid("columns that do not fit the state")),
    };
    // The same rules a row had to meet to be written.
    check_state(&state).map_err(|e| invalid(&e.to_string()))?;
    let details = PageDetails {
        problems,
        findings,
        notes,
        unconfirmed,
        injection_suspected: injection,
    };
    check_details(&details).map_err(|e| invalid(&e.to_string()))?;
    Ok(PageReading { page, state, details })
}

impl Store {
    /// Saves how far `page` of `document` has got. A page in progress is replaced by each save;
    /// a page that has ended is final, and saving over it is [`Error::PageReadingFinished`]
    /// (delete the document's readings first to read it again).
    pub async fn save_page_reading(
        &self,
        tenant: TenantId,
        document: DocId,
        page: u32,
        state: &PageState,
        details: &PageDetails,
    ) -> Result<()> {
        if page == 0 {
            return Err(Error::InvalidPageReading("pages start at 1"));
        }
        let page =
            i32::try_from(page).map_err(|_| Error::InvalidPageReading("the page number is too large"))?;
        check_state(state)?;
        check_details(details)?;
        let columns = columns(state);
        let mut tx = self.begin(tenant).await?;
        require_document(&mut *tx, tenant, document).await?;
        let saved = sqlx::query(
            "insert into page_readings \
             (tenant_id, document_id, page, state, carry, review_reason, rung, attempt, confidence_basis, \
              model, reading, problems, findings, notes, unconfirmed, injection_suspected) \
             values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) \
             on conflict (document_id, page) do update set \
               state = excluded.state, carry = excluded.carry, review_reason = excluded.review_reason, \
               rung = excluded.rung, attempt = excluded.attempt, \
               confidence_basis = excluded.confidence_basis, model = excluded.model, \
               reading = excluded.reading, problems = excluded.problems, findings = excluded.findings, \
               notes = excluded.notes, unconfirmed = excluded.unconfirmed, \
               injection_suspected = excluded.injection_suspected, updated_at = now() \
             where page_readings.state = 'in_progress'",
        )
        .bind(tenant.0)
        .bind(document.0)
        .bind(page)
        .bind(columns.state)
        .bind(columns.carry)
        .bind(columns.review_reason)
        .bind(columns.kept.map(|kept| i16::from(kept.rung)))
        // Checked above to be 1 to 50.
        .bind(columns.kept.map(|kept| kept.attempt.cast_signed()))
        .bind(columns.kept.map(|kept| kept.confidence.as_str()))
        .bind(columns.kept.map(|kept| kept.model.as_str()))
        .bind(columns.kept.map(|kept| &kept.reading))
        .bind(&details.problems)
        .bind(&details.findings)
        .bind(&details.notes)
        .bind(&details.unconfirmed)
        .bind(&details.injection_suspected)
        .execute(&mut *tx)
        .await;
        let saved = match saved {
            Ok(saved) => saved,
            // The shape was checked above, so what is left of a failed check is a size: number-heavy
            // JSON is larger in the table than as text.
            Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("23514") => {
                return Err(Error::InvalidPageReading(
                    "a reading, a carry or a list is too large for the table",
                ));
            }
            Err(error) => return Err(error.into()),
        };
        if saved.rows_affected() == 0 {
            return Err(Error::PageReadingFinished);
        }
        tx.commit().await?;
        Ok(())
    }

    /// A person's confirmation of a page that went to review: the page becomes a page that was
    /// read, with the person's reading, and the reasons it was stopped (its problems, findings,
    /// unconfirmed figures and flags) are cleared because a person has dealt with them. Only a
    /// page that is **still waiting for review** can be confirmed
    /// ([`Error::PageNotAwaitingReview`] otherwise, so two people cannot confirm the same page,
    /// and a page the model read cannot be overwritten). `notes` is what the confirmation records.
    pub async fn confirm_page_reading(
        &self,
        tenant: TenantId,
        document: DocId,
        page: u32,
        kept: &KeptReading,
        notes: &Value,
    ) -> Result<()> {
        let page_number =
            i32::try_from(page).map_err(|_| Error::InvalidPageReading("the page number is too large"))?;
        check_state(&PageState::Read(kept.clone()))?;
        if kept.confidence != ConfidenceBasis::PersonConfirmed {
            return Err(Error::InvalidPageReading(
                "a confirmation is confirmed by a person",
            ));
        }
        check_details(&PageDetails {
            notes: notes.clone(),
            ..PageDetails::default()
        })?;
        let mut tx = self.begin(tenant).await?;
        require_document(&mut *tx, tenant, document).await?;
        let updated = sqlx::query(
            "update page_readings set state = 'read', carry = null, review_reason = null, \
               rung = $4, attempt = $5, confidence_basis = $6, model = $7, reading = $8, \
               problems = '[]', findings = '[]', notes = $9, unconfirmed = '{}', \
               injection_suspected = '{}', updated_at = now() \
             where tenant_id = $1 and document_id = $2 and page = $3 and state = 'needs_review'",
        )
        .bind(tenant.0)
        .bind(document.0)
        .bind(page_number)
        .bind(i16::from(kept.rung))
        .bind(kept.attempt.cast_signed())
        .bind(kept.confidence.as_str())
        .bind(&kept.model)
        .bind(&kept.reading)
        .bind(notes)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(Error::PageNotAwaitingReview);
        }
        tx.commit().await?;
        Ok(())
    }

    /// Every stored page of `document`, in page order; empty if none has been started. Each
    /// row is checked by the rules it had to meet to be written.
    pub async fn load_page_readings(&self, tenant: TenantId, document: DocId) -> Result<Vec<PageReading>> {
        let mut tx = self.begin(tenant).await?;
        require_document(&mut *tx, tenant, document).await?;
        let rows: Vec<Row> = sqlx::query_as(
            "select page, state, carry, review_reason, rung, attempt, confidence_basis, model, reading, \
             problems, findings, notes, unconfirmed, injection_suspected \
             from page_readings where tenant_id = $1 and document_id = $2 order by page",
        )
        .bind(tenant.0)
        .bind(document.0)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter().map(from_row).collect()
    }

    /// Removes every stored page of `document` (a re-run starts from nothing) and says how many
    /// there were.
    pub async fn delete_page_readings(&self, tenant: TenantId, document: DocId) -> Result<u64> {
        let mut tx = self.begin(tenant).await?;
        require_document(&mut *tx, tenant, document).await?;
        let removed = sqlx::query("delete from page_readings where tenant_id = $1 and document_id = $2")
            .bind(tenant.0)
            .bind(document.0)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        Ok(removed)
    }
}

#[cfg(test)]
mod tests;
