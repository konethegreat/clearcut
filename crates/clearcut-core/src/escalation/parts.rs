// Written by Kone Tshivhinda & Claude
//! Splitting a dense page into parts (long-invoice completeness, B.1).
//!
//! A page is split only when it has enough words on at least two rows to be worth it, and
//! only when the split leaves every word in exactly one part, with byte ranges in text order
//! that do not overlap. Anything else plans nothing: the page is read whole, exactly as before.
//! Nothing here holds the text or reads it: the caller passes the words' byte ranges and boxes.

use serde::{Deserialize, Serialize};

/// At least this many words before a page is split.
pub const MIN_WORDS: usize = 1_200;
/// And at least this many rows.
pub const MIN_ROWS: usize = 2;
/// One part aims at this many words.
pub const WORDS_PER_PART: usize = 500;
/// A page is never split into more parts than this.
pub const MAX_PARTS: usize = 6;

/// A word as the planner needs it: where its characters are, and where it sits on the page
/// (thousandths from the top left, the same unit the worker's word boxes use).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionedWord {
    pub start: usize,
    pub end: usize,
    pub x: u16,
    pub top: u16,
    pub bottom: u16,
}

/// One part of a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedPart")]
pub struct Part {
    index: u8,
    of: u8,
    first_row: usize,
    last_row: usize,
    words: usize,
    byte_start: usize,
    byte_end: usize,
    top: u16,
    bottom: u16,
}

impl Part {
    /// 1-based.
    pub fn index(&self) -> u8 {
        self.index
    }

    pub fn of(&self) -> u8 {
        self.of
    }

    /// The rows of the page this part holds, both included.
    pub fn first_row(&self) -> usize {
        self.first_row
    }

    pub fn last_row(&self) -> usize {
        self.last_row
    }

    pub fn words(&self) -> usize {
        self.words
    }

    /// The part's text is `text[byte_start..byte_end]`.
    pub fn byte_start(&self) -> usize {
        self.byte_start
    }

    pub fn byte_end(&self) -> usize {
        self.byte_end
    }

    /// Where the part's picture begins and ends, in thousandths of the page.
    pub fn top(&self) -> u16 {
        self.top
    }

    pub fn bottom(&self) -> u16 {
        self.bottom
    }
}

/// A checked split of one page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedPlan")]
pub struct PartsPlan {
    parts: Vec<Part>,
    rows: usize,
    words: usize,
}

impl PartsPlan {
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn words(&self) -> usize {
        self.words
    }

    pub fn part(&self, index: u8) -> Option<&Part> {
        self.parts.iter().find(|part| part.index == index)
    }
}

#[derive(Deserialize)]
struct UncheckedPart {
    index: u8,
    of: u8,
    first_row: usize,
    last_row: usize,
    words: usize,
    byte_start: usize,
    byte_end: usize,
    top: u16,
    bottom: u16,
}

impl TryFrom<UncheckedPart> for Part {
    type Error = &'static str;

    fn try_from(raw: UncheckedPart) -> Result<Self, Self::Error> {
        if raw.index == 0 || raw.of < 2 || u16::from(raw.of) > MAX_PARTS as u16 {
            return Err("a part is not numbered inside a plan of two to six");
        }
        if raw.index > raw.of {
            return Err("a part's number is after the last part");
        }
        if raw.first_row > raw.last_row {
            return Err("a part's rows are upside down");
        }
        if raw.words == 0 || raw.byte_start >= raw.byte_end {
            return Err("a part holds no words");
        }
        if raw.top >= raw.bottom || raw.bottom > 1_000 {
            return Err("a part's picture range is outside the page");
        }
        Ok(Part {
            index: raw.index,
            of: raw.of,
            first_row: raw.first_row,
            last_row: raw.last_row,
            words: raw.words,
            byte_start: raw.byte_start,
            byte_end: raw.byte_end,
            top: raw.top,
            bottom: raw.bottom,
        })
    }
}

#[derive(Deserialize)]
struct UncheckedPlan {
    parts: Vec<Part>,
    rows: usize,
    words: usize,
}

impl TryFrom<UncheckedPlan> for PartsPlan {
    type Error = &'static str;

    fn try_from(raw: UncheckedPlan) -> Result<Self, Self::Error> {
        let count = raw.parts.len();
        if !(2..=MAX_PARTS).contains(&count) {
            return Err("a plan is two to six parts");
        }
        if raw.rows < count {
            return Err("a plan has fewer rows than parts");
        }
        let mut next_row = 0usize;
        let mut words = 0usize;
        for (position, part) in raw.parts.iter().enumerate() {
            if u16::from(part.of) != count as u16 || usize::from(part.index) != position + 1 {
                return Err("a plan's parts are not numbered one to its count in order");
            }
            if part.first_row != next_row || part.last_row < part.first_row {
                return Err("a plan's rows are missing, repeated or out of order");
            }
            next_row = part.last_row + 1;
            words += part.words;
            if position > 0 {
                let previous = &raw.parts[position - 1];
                if previous.byte_end > part.byte_start {
                    return Err("a plan's text ranges overlap or are out of order");
                }
                if previous.bottom != part.top {
                    return Err("a plan's picture ranges are not contiguous");
                }
            }
        }
        if next_row != raw.rows {
            return Err("a plan does not end on the page's last row");
        }
        if words != raw.words {
            return Err("a plan's words do not add up");
        }
        if raw.parts[0].top != 0 || raw.parts[count - 1].bottom != 1_000 {
            return Err("a plan does not cover the page top to bottom");
        }
        Ok(PartsPlan {
            parts: raw.parts,
            rows: raw.rows,
            words: raw.words,
        })
    }
}

/// Splits a page into parts when it is dense enough and the words allow it; otherwise `None`.
pub fn plan_parts(words: &[PositionedWord], text_len: usize) -> Option<PartsPlan> {
    if words.len() < MIN_WORDS
        || words
            .iter()
            .any(|word| word.start >= word.end || word.end > text_len || word.top >= word.bottom)
    {
        return None;
    }
    // Words sorted top to bottom, left to right, then by their place in the text.
    let mut order: Vec<usize> = (0..words.len()).collect();
    order.sort_by_key(|&index| (words[index].top, words[index].x, words[index].start));
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut row_bottom = 0u32;
    for index in order {
        let word = words[index];
        if rows.is_empty() || u32::from(word.top) >= row_bottom {
            rows.push(vec![index]);
            row_bottom = u32::from(word.bottom);
        } else {
            rows.last_mut()?.push(index);
            row_bottom = row_bottom.max(u32::from(word.bottom));
        }
    }
    if rows.len() < MIN_ROWS {
        return None;
    }
    let wanted = words.len().div_ceil(WORDS_PER_PART).clamp(2, MAX_PARTS);
    let count = wanted.min(rows.len());
    let base = rows.len() / count;
    let remainder = rows.len() % count;

    struct Draft {
        first_row: usize,
        last_row: usize,
        words: usize,
        byte_start: usize,
        byte_end: usize,
        top: u16,
        bottom: u16,
    }
    let mut drafts = Vec::with_capacity(count);
    let mut first_row = 0usize;
    for position in 0..count {
        let row_count = base + usize::from(position < remainder);
        let last_row = first_row + row_count - 1;
        let mut byte_start = usize::MAX;
        let mut byte_end = 0usize;
        let mut count_words = 0usize;
        let mut top = u16::MAX;
        let mut bottom = 0u16;
        for row in &rows[first_row..=last_row] {
            for &index in row {
                let word = words[index];
                byte_start = byte_start.min(word.start);
                byte_end = byte_end.max(word.end);
                count_words += 1;
                top = top.min(word.top);
                bottom = bottom.max(word.bottom);
            }
        }
        drafts.push(Draft {
            first_row,
            last_row,
            words: count_words,
            byte_start,
            byte_end,
            top,
            bottom,
        });
        first_row = last_row + 1;
    }

    let mut parts = Vec::with_capacity(count);
    for (position, draft) in drafts.iter().enumerate() {
        let index = u8::try_from(position + 1).ok()?;
        let of = u8::try_from(count).ok()?;
        let top = if position == 0 {
            0
        } else {
            midpoint(drafts[position - 1].bottom, draft.top)
        };
        let bottom = if position + 1 == count {
            1_000
        } else {
            midpoint(draft.bottom, drafts[position + 1].top)
        };
        parts.push(Part {
            index,
            of,
            first_row: draft.first_row,
            last_row: draft.last_row,
            words: draft.words,
            byte_start: draft.byte_start,
            byte_end: draft.byte_end,
            top,
            bottom,
        });
    }
    PartsPlan::try_from(UncheckedPlan {
        parts,
        rows: rows.len(),
        words: words.len(),
    })
    .ok()
}

fn midpoint(low: u16, high: u16) -> u16 {
    let sum = u32::from(low) + u32::from(high);
    u16::try_from(sum / 2).unwrap_or(u16::MAX)
}
