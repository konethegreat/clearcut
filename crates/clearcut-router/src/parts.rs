// Written by Kone Tshivhinda & Claude
//! A page read in parts: the plan, the parts already done, and what one part came to.
//!
//! The carry is stored as JSON between two runs of a page, so it is checked when read back
//! (`#[serde(try_from = ...)]`): a damaged plan, a part out of order, a part that claims no
//! call, or calls that do not add up stop the resume instead of spending calls twice.

use clearcut_core::escalation::{PageOutcome, PartsPlan};
use clearcut_core::reading::PageReading;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ReadingOrigin;

/// The calls a page may use for each part after its first, so that splitting a page never
/// takes a later page's calls.
pub const EXTRA_CALLS_PER_PART: u32 = 3;

/// One part that has been read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartDone {
    /// 1-based.
    pub index: u8,
    pub outcome: PageOutcome,
    /// The part's reading (the kept one), for the merge.
    pub reading: Option<Value>,
    /// Which rung, attempt and model gave `reading`.
    pub reading_from: Option<ReadingOrigin>,
    /// The calls this part's reader issued (sent or not).
    pub issued: u32,
}

/// A page being read in parts, as it is stored between two runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedPartsCarry")]
pub struct PartsCarry {
    pub plan: PartsPlan,
    /// How many parts are done; the part now being read is `index + 1`.
    pub index: u32,
    pub done: Vec<PartDone>,
    /// The attempts issued on the page before the current part's reader started.
    pub issued_before: u32,
    /// Instruction-like kinds seen in the parts read so far (never the text).
    #[serde(default)]
    pub injection: Vec<String>,
}

#[derive(Deserialize)]
struct UncheckedPartsCarry {
    plan: PartsPlan,
    index: u32,
    done: Vec<PartDone>,
    issued_before: u32,
    #[serde(default)]
    injection: Vec<String>,
}

impl TryFrom<UncheckedPartsCarry> for PartsCarry {
    type Error = &'static str;

    fn try_from(raw: UncheckedPartsCarry) -> Result<Self, Self::Error> {
        let planned = raw.plan.parts().len();
        if raw.index as usize != raw.done.len() {
            return Err("the parts done do not match the part index");
        }
        if raw.index as usize >= planned {
            return Err("every part of the page is done");
        }
        for (position, part) in raw.done.iter().enumerate() {
            if usize::from(part.index) != position + 1 {
                return Err("a part is out of order");
            }
            if part.issued == 0 {
                return Err("a part claims no call");
            }
            if let PageOutcome::Read { .. } = &part.outcome {
                let Some(reading) = &part.reading else {
                    return Err("a read part has no reading");
                };
                if PageReading::from_stored_value(reading.clone()).is_err() {
                    return Err("a part's reading is not a reading");
                }
                if part.reading_from.is_none() {
                    return Err("a read part has no origin");
                }
            }
        }
        let issued: u32 = raw.done.iter().map(|part| part.issued).sum();
        if issued != raw.issued_before {
            return Err("the calls of the parts done do not add up");
        }
        if raw.issued_before > 64 {
            return Err("a page has more calls than a page can make");
        }
        Ok(PartsCarry {
            plan: raw.plan,
            index: raw.index,
            done: raw.done,
            issued_before: raw.issued_before,
            injection: raw.injection,
        })
    }
}
