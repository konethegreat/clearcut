// Written by Kone Tshivhinda & Claude
//! Reading a page reading that was stored before the current shape (ADR-0022).
//!
//! A page reading is stored as JSON in `page_readings`, with no shape version of its own. The
//! shape changed from `reading-v2` (no carried figures) to `reading-v3` (ADR-0022), so a reading
//! written before the change would no longer deserialize into [`PageReading`]. Rather than
//! leaving those documents unreadable, a frozen copy of the v2 shape sits here: a stored reading
//! is read as v3 first, and as v2 (lifted to v3 with empty carried figures) if that fails.
//!
//! This is the only place that knows the old shape. A future shape change should freeze v3 the
//! same way rather than weaken the model-facing schema.

use super::{
    PageReading, ReadCarried, ReadCreditNote, ReadCurrency, ReadDue, ReadParty, ReadTotals, ReadVatBasis,
};
use serde::Deserialize;

/// The prompt version a reading written under `reading-v2` came from. Kept so a converted old
/// reading does not claim it was produced by the current prompt.
pub const LEGACY_PROMPT_VERSION: &str = "reading-v2/prompt-v1";

/// A stored reading and which shape it was written under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredReading {
    pub reading: PageReading,
    /// True when the JSON was the older `reading-v2` shape.
    pub legacy: bool,
}

impl PageReading {
    /// Reads a reading out of stored JSON: the current shape first, then the frozen v2 shape.
    ///
    /// Returns the current shape's error when neither fits, so a damaged reading is refused
    /// rather than guessed at.
    pub fn from_stored_value(value: serde_json::Value) -> Result<StoredReading, serde_json::Error> {
        match serde_json::from_value::<PageReading>(value.clone()) {
            Ok(reading) => Ok(StoredReading {
                reading,
                legacy: false,
            }),
            // The current shape refused it; try the shape that was current before ADR-0022.
            Err(current_error) => match serde_json::from_value::<ReadingV2>(value) {
                Ok(old) => Ok(StoredReading {
                    reading: old.into(),
                    legacy: true,
                }),
                // Report the current shape's error: it names what a caller can fix today.
                Err(_) => Err(current_error),
            },
        }
    }
}

/// The `reading-v2` shape, frozen. Everything except the carried figures is the same type as
/// today, so only the outer object needs its own copy.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadingV2 {
    currency: ReadCurrency,
    amounts_include_vat: ReadVatBasis,
    invoice_number: String,
    invoice_date: String,
    supplier: ReadParty,
    recipient: ReadParty,
    lines: Vec<super::ReadLine>,
    totals: ReadTotals,
    due: ReadDue,
    credit_note: ReadCreditNote,
}

impl From<ReadingV2> for PageReading {
    fn from(old: ReadingV2) -> Self {
        PageReading {
            currency: old.currency,
            amounts_include_vat: old.amounts_include_vat,
            invoice_number: old.invoice_number,
            invoice_date: old.invoice_date,
            supplier: old.supplier,
            recipient: old.recipient,
            lines: old.lines,
            totals: old.totals,
            carried: ReadCarried::default(),
            due: old.due,
            credit_note: old.credit_note,
        }
    }
}
