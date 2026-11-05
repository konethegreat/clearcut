# Long invoices, Slice B.1: a dense page is read in parts

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A page with more lines than one model call can report is split, from its own words, into two to six overlapping-free parts; each part is read and escalated on its own; the parts are merged into one page reading, with disagreement, a failed part and a page that is still too dense each ending in a person's queue.

**Architecture:** A pure planner in `clearcut-core` groups the page's words into rows and rows into balanced parts, and a pure merge joins the part readings. The worker learns to draw a vertical crop of a page, the document processor carries the crop in `RenderOptions` and maps redaction boxes into a part's picture, and the pack labels a part in the context, the instruction and the manifest under a new `prompt-v4`. The router plans a new page after fetching its words, runs each part through its own `PageReader` (attempt numbers stay page-global), merges the readings and evaluates the merge once against the whole page text. The carry stores the plan and the parts already done. The pipeline records the parts evidence in the page's notes and widens the document's call allowance by three calls for every part after the first.

**Tech Stack:** Rust workspace (`clearcut-core`, `clearcut-docproc`, `clearcut-pack`, `clearcut-router`, `clearcut-pipeline`), Python 3.12/3.14 worker (FastAPI, Pillow, pdfium), TypeScript SDK doc comment, Markdown docs.

**Spec:** [docs/superpowers/specs/2026-11-03-long-invoice-completeness-design.md](../specs/2026-11-03-long-invoice-completeness-design.md) (Slice B.1, §6.1 to §6.4, §10, §12).

## Global Constraints

- **One additive migration, and no other schema change.** `page_readings_review_reason_check` (migration 0008) lists the ten reasons that existed then, so the three new reasons cannot be stored until it is widened. Migration `0013_review_reason_parts.sql` drops and re-adds that one check with the full `ReviewReason::ALL` list; it changes no data and no column. Applied migrations are never edited. Everything else stays JSON: `Carry.parts` is additive with `#[serde(default)]`, and a carry stored before this slice resumes as a whole-page carry.
- **The reading schema stays `reading-v3`.** `MAX_LINES_PER_PAGE` becomes 600 (still `reading-v3`: the shape and its field descriptions do not change, only the storage cap). The prompt becomes `prompt-v4`, so `prompt_version()` returns `reading-v3/prompt-v4`.
- **Money is integer cents; no new price and no new model.** A part costs the same as any page call.
- **No document content in a log, audit entry, note, error or test fixture.** Parts evidence carries indexes, seams in per mille, a fixed reason and counts only. Uploaded text and model output are untrusted data, never instructions.
- **A part's picture is a crop at the rung's dpi, and `max_edge` applies to the crop.** The worker crops after drawing and before encoding.
- **An unverified text layer is never painted from.** Parts of an `ImageFirst` page are sent with `unverified_text_layer` and checked against their text as `TextTrust::Unverified`.
- **A page is split only when it has at least 1 200 words arranged in at least two rows.** A page below that is read whole exactly as before.
- **User-facing copy contains no em dashes**, uses light surfaces, and says plainly what the system does.
- **Every source file keeps its first line** `// Written by Kone Tshivhinda & Claude`.
- **Test first.** Each task writes the failing test, runs it, implements, runs it green, then commits. Conventional commits.
- **Never print, commit or test with a real API key.** All tests use fakes and throwaway tenants. **Never upload a document to the live stack.**
- **Every gate green before merge:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (with the test Postgres), the web tests, the worker tests (`ruff`, `mypy`, `pytest` on 3.12 and 3.14), the contract check and the gold-set check, as `README.md` and CI define them.

### Settled rulings (do not reopen while executing)

1. **Part trigger and count.** Trigger at `words >= 1_200 && rows >= 2`. Part count `ceil(words / 500).clamp(2, 6)`, then capped at the row count. Above 3 000 words the count is six. An invalid layout (parts' byte ranges out of order or overlapping) plans nothing: the page is read whole and, if it truncates, ends `too_long`.
2. **Seams.** A part's `top` is 0 for the first part and the midpoint between the previous part's lowest word bottom and this part's highest word top otherwise; `bottom` is 1000 for the last part and the same kind of midpoint otherwise. Tops are contiguous: `part[i].bottom == part[i+1].top`. Slice B.1 never marks a seam uncertain; `seam_uncertain` is always `false` until B.2.
3. **Totals are not checked per part, and the merged page is a whole document only when it is the only page.** `assess` already counts totals findings only when `whole_document` is true; a part always passes `false`. The merged reading is assessed once against the whole page text with `whole_document = (page count == 1)`, which is exactly what a whole page gets today: on a one-page document that assessment is the whole-document check, and assembly runs the document's own check again after the pages are put together.
4. **Per-part problems and findings are not unioned** (line numbers restart in each part). The merged reading is converted, validated and assessed once against the whole page text, and that evaluation is what the page stores.
5. **The merged page is `Read` only if** every part is `Read`, the merge has no conflict, the merged assessment is `Accept`, the merged reading holds at most 600 lines, and its JSON is at most `MAX_STORED_READING_BYTES` (240 000). Confidence is the lowest part confidence.
6. **The stored JSON guard.** The spec's claim that 600 longest lines fit the store's 250 KB is false (about 358 KB), so the router refuses a merged reading over 240 000 JSON bytes and ends `too_dense` with no draft. This is a documented deviation from the spec text.
7. **Reason precedence on a parts page:** `PartConflict` > `TooDense` > `Budget` > `CallCap` > `PartFailed`. A part that ends `Budget` or `CallCap` propagates that reason. A part still cut off at the truncation ceiling ends the page `TooDense` (the spec's "if a part then still truncates, the page ends `too_dense`": splitting further is not allowed). Any other part review ends `PartFailed`.
8. **Call allowance.** `EXTRA_CALLS_PER_PART = 3`. A page planned into `n` parts may use the document's remaining allowance plus `3 * (n - 1)`; each part has its own 5-call cap. The pipeline adds the same extras from stored evidence so later pages keep their allowance.
9. **Prompt v4 adds one rule** about reading only what is visible in a part. The v3 prompt files and golden are removed, not kept.
10. **No SDK or OpenAPI enum change.** The review reason is a string in both; only the SDK doc comment learns the three new names.

---

### Task 1: The pure planner in `clearcut-core`

**Files:**
- Create: `crates/clearcut-core/src/escalation/parts.rs`
- Create: `crates/clearcut-core/src/escalation/tests_parts.rs`
- Modify: `crates/clearcut-core/src/escalation/mod.rs` (module and re-export)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub const MIN_WORDS: usize = 1_200;`, `pub const MIN_ROWS: usize = 2;`, `pub const WORDS_PER_PART: usize = 500;`, `pub const MAX_PARTS: usize = 6;`
  - `pub struct PositionedWord { pub start: usize, pub end: usize, pub x: u16, pub top: u16, pub bottom: u16 }`
  - `pub struct Part { index: u8, of: u8, first_row: usize, last_row: usize, words: usize, byte_start: usize, byte_end: usize, top: u16, bottom: u16 }` with `index()`, `of()`, `first_row()`, `last_row()`, `words()`, `byte_start()`, `byte_end()`, `top()`, `bottom()`
  - `pub struct PartsPlan { parts: Vec<Part>, rows: usize, words: usize }` with `parts()`, `rows()`, `words()`, `part(index: u8) -> Option<&Part>`
  - `pub fn plan_parts(words: &[PositionedWord], text_len: usize) -> Option<PartsPlan>`
  - Both `Part` and `PartsPlan` are `Serialize`, `Deserialize` with `#[serde(try_from = "...")]` and are refused when damaged.

- [ ] **Step 1: Write the failing tests**

Create `crates/clearcut-core/src/escalation/tests_parts.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! The planner: when a page is split, how many parts, and whether every word lands in one.

use super::parts::{plan_parts, PositionedWord, MAX_PARTS, MIN_WORDS};
use serde_json::{json, Value};

/// `count` words of the same width, `per_row` to a row, in reading order.
fn layout(count: usize, per_row: usize) -> (String, Vec<PositionedWord>) {
    let mut text = String::new();
    let mut words = Vec::new();
    for i in 0..count {
        if i > 0 {
            text.push(' ');
        }
        let start = text.len();
        text.push_str(&format!("w{i:05}"));
        let end = text.len();
        let row = i / per_row;
        let column = i % per_row;
        words.push(PositionedWord {
            start,
            end,
            x: u16::try_from((column % 25) * 38).unwrap(),
            top: u16::try_from(row * 30).unwrap(),
            bottom: u16::try_from(row * 30 + 20).unwrap(),
        });
    }
    (text, words)
}

#[test]
fn a_page_below_twelve_hundred_words_is_not_split() {
    let (text, words) = layout(MIN_WORDS - 1, 40);
    assert!(plan_parts(&words, text.len()).is_none());
}

#[test]
fn a_page_with_one_row_is_not_split() {
    let (text, words) = layout(1_200, 1_200);
    assert!(plan_parts(&words, text.len()).is_none());
}

#[test]
fn twelve_hundred_words_in_thirty_rows_are_three_balanced_parts() {
    let (text, words) = layout(1_200, 40);
    let plan = plan_parts(&words, text.len()).unwrap();
    assert_eq!(plan.parts().len(), 3);
    assert_eq!(plan.rows(), 30);
    assert_eq!(plan.words(), 1_200);
    let shape: Vec<_> = plan
        .parts()
        .iter()
        .map(|p| (p.index(), p.of(), p.first_row(), p.last_row(), p.words()))
        .collect();
    assert_eq!(shape, vec![(1, 3, 0, 9, 400), (2, 3, 10, 19, 400), (3, 3, 20, 29, 400)]);
    assert_eq!(plan.parts()[0].top(), 0);
    assert_eq!(plan.parts()[2].bottom(), 1_000);
    // Row 9 ends at 290, row 10 begins at 300: the seam is the midpoint.
    assert_eq!(plan.parts()[0].bottom(), 295);
    assert_eq!(plan.parts()[1].top(), 295);
    assert_eq!(plan.parts()[1].bottom(), 595);
    assert_eq!(plan.parts()[2].top(), 595);
}

#[test]
fn twelve_hundred_words_in_two_rows_are_two_parts_of_one_row_each() {
    let (text, words) = layout(1_200, 600);
    let plan = plan_parts(&words, text.len()).unwrap();
    assert_eq!(plan.parts().len(), 2);
    assert_eq!(
        plan.parts().iter().map(|p| p.words()).collect::<Vec<_>>(),
        vec![600, 600]
    );
}

#[test]
fn five_thousand_words_are_six_parts() {
    let (text, words) = layout(5_000, 40);
    let plan = plan_parts(&words, text.len()).unwrap();
    assert_eq!(plan.parts().len(), MAX_PARTS);
}

#[test]
fn every_word_lands_in_one_part_and_the_ranges_are_ordered_and_do_not_overlap() {
    for (count, per_row) in [(1_200, 40), (2_400, 60), (7_000, 50)] {
        let (text, words) = layout(count, per_row);
        let plan = plan_parts(&words, text.len()).unwrap();
        let mut seen = vec![0usize; words.len()];
        let mut ranges = Vec::new();
        for part in plan.parts() {
            let mut inside = 0;
            for (index, word) in words.iter().enumerate() {
                if word.start >= part.byte_start() && word.end <= part.byte_end() {
                    inside += 1;
                    seen[index] += 1;
                }
            }
            assert_eq!(inside, part.words(), "part {}", part.index());
            ranges.push((part.byte_start(), part.byte_end()));
        }
        assert_eq!(seen, vec![1usize; words.len()]);
        for pair in ranges.windows(2) {
            assert!(pair[0].1 <= pair[1].0, "{ranges:?}");
            // Only white space may lie between two parts.
            assert!(text[pair[0].1..pair[1].0].trim().is_empty());
        }
    }
}

#[test]
fn a_layout_whose_text_ranges_would_overlap_is_not_split() {
    // Two columns read top to bottom, but the text runs down the left column first: the
    // first part's range would run past the second part's start.
    let text = "A B C D".to_string();
    let word = |start: usize, top: u16| PositionedWord {
        start,
        end: start + 1,
        x: if start < 4 { 100 } else { 600 },
        top,
        bottom: top + 20,
    };
    let words = vec![word(0, 10), word(2, 100), word(4, 10), word(6, 100)];
    assert!(plan_parts(&words, text.len()).is_none());
}

#[test]
fn a_plan_read_back_from_text_is_checked() {
    let (text, words) = layout(1_200, 40);
    let plan = plan_parts(&words, text.len()).unwrap();
    let value = serde_json::to_value(&plan).unwrap();
    let back: super::parts::PartsPlan = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(back, plan);

    let mut missing = value.clone();
    missing["parts"].as_array_mut().unwrap().pop();
    assert!(serde_json::from_value::<super::parts::PartsPlan>(missing).is_err());

    let mut wrong_of = value.clone();
    wrong_of["parts"][0]["of"] = json!(4);
    assert!(serde_json::from_value::<super::parts::PartsPlan>(wrong_of).is_err());

    let mut overlapping = value.clone();
    overlapping["parts"][1]["byte_start"] = json!(0);
    assert!(serde_json::from_value::<super::parts::PartsPlan>(overlapping).is_err());

    let mut upside_down = Value::clone(&value);
    upside_down["parts"][0]["top"] = json!(600);
    assert!(serde_json::from_value::<super::parts::PartsPlan>(upside_down).is_err());
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p clearcut-core escalation::tests_parts`
Expected: FAIL, `unresolved import super::parts`.

- [ ] **Step 3: Write the planner**

Create `crates/clearcut-core/src/escalation/parts.rs`:

```rust
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
```

Notes for the implementer: `PartsPlan::try_from(UncheckedPlan { .. })` is called directly (both live in this module), so the same checks that guard a stored plan also guard a freshly built one. Keep `UncheckedPlan` and `UncheckedPart` private.

- [ ] **Step 4: Register the module**

In `crates/clearcut-core/src/escalation/mod.rs`, add beside the other modules:

```rust
mod parts;

#[cfg(test)]
mod tests_parts;
```

and beside the other re-exports:

```rust
pub use parts::{plan_parts, Part, PartsPlan, PositionedWord};
```

- [ ] **Step 5: Run the tests until green**

Run: `cargo test -p clearcut-core escalation::tests_parts`
Expected: PASS (7 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/clearcut-core/src/escalation/parts.rs crates/clearcut-core/src/escalation/tests_parts.rs crates/clearcut-core/src/escalation/mod.rs
git commit -m "feat(core): plan a dense page into parts from its words"
```

---

### Task 2: Merge part readings, the new reasons, and 600 lines

**Files:**
- Create: `crates/clearcut-core/src/escalation/merge.rs`
- Create: `crates/clearcut-core/src/escalation/tests_merge.rs`
- Modify: `crates/clearcut-core/src/escalation/mod.rs`
- Modify: `crates/clearcut-core/src/escalation/assess.rs:93`
- Modify: `crates/clearcut-core/src/escalation/tests_assess.rs:182,357,403`
- Modify: `crates/clearcut-core/src/escalation/tests_policy.rs`
- Modify: `crates/clearcut-core/src/reading/mod.rs:38`
- Modify: `crates/clearcut-core/src/reading/tests.rs:762`
- Modify: `crates/clearcut-core/src/register/mod.rs:780`
- Create: `crates/clearcut-store/migrations/0013_review_reason_parts.sql`

**Interfaces:**
- Consumes: `PageReading`, `ReadLine`, `ReadParty`, `ReadTotals`, `ReadCarried`, `ReadDue`, `ReadCreditNote`, `ReadCurrency`, `ReadVatBasis`.
- Produces:
  - `pub fn merge_parts(readings: &[PageReading]) -> Result<Merged, MergeError>`
  - `pub struct Merged { pub reading: PageReading, pub conflicts: Vec<&'static str> }`
  - `pub enum MergeError { NoParts }`
  - `ReviewReason::{PartConflict, PartFailed, TooDense}` with names `part_conflict`, `part_failed`, `too_dense`
  - `MAX_LINES_PER_PAGE = 600`
  - `ProblemKind::TooManyLines` now assesses to `ReviewReason::TooDense`

- [ ] **Step 1: Write the failing merge tests**

Create `crates/clearcut-core/src/escalation/tests_merge.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! Merging the parts of one page: lines in order, facts once, disagreement recorded.

use super::merge::{merge_parts, MergeError};
use crate::reading::{
    PageReading, ReadCarried, ReadCreditNote, ReadCurrency, ReadDue, ReadLine, ReadParty, ReadTax,
    ReadTotals, ReadVatBasis,
};

fn line(description: &str, total: &str) -> ReadLine {
    ReadLine {
        description: description.into(),
        quantity: "1".into(),
        unit_price: total.into(),
        line_total: total.into(),
        tax: ReadTax::Standard,
    }
}

fn page(lines: Vec<ReadLine>) -> PageReading {
    PageReading {
        currency: ReadCurrency::Zar,
        amounts_include_vat: ReadVatBasis::No,
        invoice_number: String::new(),
        invoice_date: String::new(),
        supplier: ReadParty::default(),
        recipient: ReadParty::default(),
        lines,
        totals: ReadTotals::default(),
        carried: ReadCarried::default(),
        due: ReadDue::default(),
        credit_note: ReadCreditNote::default(),
    }
}

#[test]
fn no_parts_is_an_error() {
    assert_eq!(merge_parts(&[]), Err(MergeError::NoParts));
}

#[test]
fn lines_are_joined_in_part_order() {
    let merged = merge_parts(&[
        page(vec![line("a", "1.00"), line("b", "2.00")]),
        page(vec![line("c", "3.00")]),
    ])
    .unwrap();
    let descriptions: Vec<_> = merged
        .reading
        .lines
        .iter()
        .map(|line| line.description.as_str())
        .collect();
    assert_eq!(descriptions, vec!["a", "b", "c"]);
    assert!(merged.conflicts.is_empty());
}

#[test]
fn a_fact_is_taken_from_the_first_part_that_shows_it() {
    let mut first = page(vec![]);
    first.invoice_number = "INV-1".into();
    let mut second = page(vec![]);
    second.invoice_number = "INV-1".into();
    let merged = merge_parts(&[first, second]).unwrap();
    assert_eq!(merged.reading.invoice_number, "INV-1");
    assert!(merged.conflicts.is_empty());
}

#[test]
fn a_later_different_fact_is_a_conflict_and_the_first_one_stays() {
    let mut first = page(vec![]);
    first.invoice_number = "INV-1".into();
    first.supplier.name = "Acme".into();
    first.totals.gross = "10.00".into();
    let mut second = page(vec![]);
    second.invoice_number = "INV-2".into();
    second.supplier.name = "Other".into();
    second.totals.gross = "11.00".into();
    let merged = merge_parts(&[first, second]).unwrap();
    assert_eq!(merged.reading.invoice_number, "INV-1");
    assert_eq!(merged.reading.supplier.name, "Acme");
    assert_eq!(merged.reading.totals.gross, "10.00");
    assert_eq!(
        merged.conflicts,
        vec!["invoice_number", "supplier/name", "totals/gross"]
    );
}

#[test]
fn an_empty_value_and_not_shown_are_not_disagreements() {
    let mut first = page(vec![]);
    first.amounts_include_vat = ReadVatBasis::NotShown;
    first.due.terms = "30 days".into();
    let mut second = page(vec![]);
    second.amounts_include_vat = ReadVatBasis::No;
    second.due.terms = String::new();
    let merged = merge_parts(&[first, second]).unwrap();
    assert_eq!(merged.reading.amounts_include_vat, ReadVatBasis::No);
    assert_eq!(merged.reading.due.terms, "30 days");
    assert!(merged.conflicts.is_empty());
}

#[test]
fn each_total_is_merged_on_its_own() {
    let mut first = page(vec![]);
    first.totals.net = "10.00".into();
    let mut second = page(vec![]);
    second.totals.gross = "11.50".into();
    let merged = merge_parts(&[first, second]).unwrap();
    assert_eq!(merged.reading.totals.net, "10.00");
    assert_eq!(merged.reading.totals.gross, "11.50");
    assert!(merged.conflicts.is_empty());
}

#[test]
fn a_conflict_is_named_once() {
    let mut first = page(vec![]);
    first.recipient.vat_number = "1".into();
    let mut second = page(vec![]);
    second.recipient.vat_number = "2".into();
    second.invoice_date = "2026-01-01".into();
    let mut third = page(vec![]);
    third.recipient.vat_number = "3".into();
    third.invoice_date = "2026-01-02".into();
    let merged = merge_parts(&[first, second, third]).unwrap();
    assert_eq!(
        merged.conflicts,
        vec!["recipient/vat_number", "invoice_date"]
    );
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p clearcut-core escalation::tests_merge`
Expected: FAIL, `unresolved import super::merge`.

- [ ] **Step 3: Write the merge**

Create `crates/clearcut-core/src/escalation/merge.rs`:

```rust
// Written by Kone Tshivhinda & Claude
//! Joining the parts of one page (long-invoice completeness, B.1).
//!
//! Lines are concatenated in part order. Every other fact is taken from the first part that
//! shows it; a later part that shows a different value is a conflict, never a repair. Nothing
//! here reads text or guesses: the caller converts, validates and assesses the merged reading.

use crate::reading::{PageReading, ReadParty, ReadTotals};

/// Why no merge could be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MergeError {
    #[error("there are no parts to merge")]
    NoParts,
}

/// The merged page, and the facts the parts disagreed on (fixed paths, in first-seen order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub reading: PageReading,
    pub conflicts: Vec<&'static str>,
}

fn take<T: Clone + Default + PartialEq>(slot: &mut T, later: &T, path: &'static str, conflicts: &mut Vec<&'static str>) {
    if slot == &T::default() {
        *slot = later.clone();
    } else if slot != later && later != &T::default() && !conflicts.contains(&path) {
        conflicts.push(path);
    }
}

fn take_text(slot: &mut String, later: &str, path: &'static str, conflicts: &mut Vec<&'static str>) {
    if slot.is_empty() {
        slot.push_str(later);
    } else if slot != later && !later.is_empty() && !conflicts.contains(&path) {
        conflicts.push(path);
    }
}

/// Merges the parts of one page, in the order they were read.
pub fn merge_parts(readings: &[PageReading]) -> Result<Merged, MergeError> {
    let Some((first, rest)) = readings.split_first() else {
        return Err(MergeError::NoParts);
    };
    let mut merged = first.clone();
    let mut conflicts = Vec::new();
    for part in rest {
        merged.lines.extend(part.lines.iter().cloned());
        take_text(&mut merged.invoice_number, &part.invoice_number, "invoice_number", &mut conflicts);
        take_text(&mut merged.invoice_date, &part.invoice_date, "invoice_date", &mut conflicts);
        take(&mut merged.currency, &part.currency, "currency", &mut conflicts);
        take(
            &mut merged.amounts_include_vat,
            &part.amounts_include_vat,
            "amounts_include_vat",
            &mut conflicts,
        );
        take_party(&mut merged.supplier, &part.supplier, "supplier", &mut conflicts);
        take_party(&mut merged.recipient, &part.recipient, "recipient", &mut conflicts);
        take_totals(&mut merged.totals, &part.totals, &mut conflicts);
        take_text(
            &mut merged.carried.brought_forward,
            &part.carried.brought_forward,
            "carried/brought_forward",
            &mut conflicts,
        );
        take_text(
            &mut merged.carried.carried_forward,
            &part.carried.carried_forward,
            "carried/carried_forward",
            &mut conflicts,
        );
        take_text(&mut merged.due.date, &part.due.date, "due/date", &mut conflicts);
        take_text(&mut merged.due.terms, &part.due.terms, "due/terms", &mut conflicts);
        take_text(
            &mut merged.credit_note.original_invoice_number,
            &part.credit_note.original_invoice_number,
            "credit_note/original_invoice_number",
            &mut conflicts,
        );
        take_text(
            &mut merged.credit_note.amount,
            &part.credit_note.amount,
            "credit_note/amount",
            &mut conflicts,
        );
    }
    Ok(Merged {
        reading: merged,
        conflicts,
    })
}
```

Add the helpers below `merge_parts`:

```rust
fn take_party(
    slot: &mut ReadParty,
    later: &ReadParty,
    prefix: &'static str,
    conflicts: &mut Vec<&'static str>,
) {
    let path = |field: &'static str| -> &'static str {
        match (prefix, field) {
            ("supplier", "name") => "supplier/name",
            ("supplier", "address") => "supplier/address",
            _ if prefix == "supplier" => "supplier/vat_number",
            ("recipient", "name") => "recipient/name",
            ("recipient", "address") => "recipient/address",
            _ => "recipient/vat_number",
        }
    };
    take_text(&mut slot.name, &later.name, path("name"), conflicts);
    take_text(&mut slot.address, &later.address, path("address"), conflicts);
    take_text(&mut slot.vat_number, &later.vat_number, path("vat_number"), conflicts);
}

fn take_totals(slot: &mut ReadTotals, later: &ReadTotals, conflicts: &mut Vec<&'static str>) {
    take_text(&mut slot.net, &later.net, "totals/net", conflicts);
    take_text(&mut slot.vat, &later.vat, "totals/vat", conflicts);
    take_text(&mut slot.gross, &later.gross, "totals/gross", conflicts);
}
```

- [ ] **Step 4: Write the failing reason tests**

In `crates/clearcut-core/src/escalation/tests_policy.rs`, extend the name list:

```rust
            "amounts_include_vat",
            "tax_not_shown",
            "part_conflict",
            "part_failed",
            "too_dense",
```

In `crates/clearcut-core/src/escalation/tests_assess.rs`, change the three `TooLong` expectations that come from `ProblemKind::TooManyLines` to `ReviewReason::TooDense`:

- `a_page_with_too_many_lines_goes_to_a_person_without_escalating` (line 182).
- the `(problem("lines", ProblemKind::TooManyLines), ReviewReason::TooLong)` pair in `a_review_problem_counts_as_blocking` (line 357).
- `a_page_that_is_too_long_stays_too_long_with_other_problems_around_it` (line 403); rename it `a_page_that_is_too_dense_stays_too_dense_with_other_problems_around_it`.

In `crates/clearcut-core/src/reading/tests.rs`, rename and change `at_most_two_hundred_lines_are_kept_and_the_rest_reported` (line 762) to 600/601:

```rust
#[test]
fn at_most_six_hundred_lines_are_kept_and_the_rest_reported() {
    let many = |n: usize| PageReading {
        lines: (0..n)
            .map(|i| line(&format!("l{i}"), "1", "1.00", "1.00", ReadTax::Standard))
            .collect(),
        ..clean_reading()
    };
    let fine = convert(&many(600));
    assert_eq!(fine.extraction.lines.len(), 600);
    assert!(fine.problems.is_empty());

    let over = convert(&many(601));
    assert_eq!(over.extraction.lines.len(), 600);
    assert_eq!(over.extraction.lines[599].value.description, "l599");
    assert_eq!(
        over.problems
            .iter()
            .map(|p| (p.path.clone(), p.kind))
            .collect::<Vec<_>>(),
        vec![path("lines", ProblemKind::TooManyLines)]
    );
}
```

- [ ] **Step 5: Implement the reasons and the cap**

In `mod.rs`:

```rust
mod merge;
#[cfg(test)]
mod tests_merge;
pub use merge::{merge_parts, MergeError, Merged};
```

In `ReviewReason`, add after `TooLong`:

```rust
    /// The parts of one page read different values in the same place.
    PartConflict,
    /// One part of a page could not be read.
    PartFailed,
    /// More lines than one page can hold (600, or more JSON than the store takes).
    TooDense,
```

Update `ALL` to `[ReviewReason; 13]` with the new variants appended after `TaxNotShown`, and `as_str`:

```rust
            ReviewReason::PartConflict => "part_conflict",
            ReviewReason::PartFailed => "part_failed",
            ReviewReason::TooDense => "too_dense",
```

In `reading/mod.rs`:

```rust
/// At most this many lines are kept from one page.
pub const MAX_LINES_PER_PAGE: usize = 600;
```

In `assess.rs`, the `TooManyLines` arm becomes:

```rust
            ProblemKind::TooManyLines => {
                review = review.or(Some(ReviewReason::TooDense));
                blocking += 1;
            }
```

In `register/mod.rs` `why()`, add:

```rust
        ReviewReason::PartConflict => "The parts of this page do not agree on a value.",
        ReviewReason::PartFailed => "One part of this page could not be read.",
        ReviewReason::TooDense => "This page has more lines than one page can hold.",
```

- [ ] **Step 6: Run the tests until green**

Run: `cargo test -p clearcut-core`
Expected: PASS. If a register snapshot lists review sentences, update it with the three new lines; do not change any existing sentence.

- [ ] **Step 7: Widen the stored review-reason check**

The three new reasons are rejected by `page_readings_review_reason_check` until it is widened, and `every_review_reason_and_every_basis_survives_the_round_trip` proves it. Migration 0008 is applied and checksummed; never edit it. Create a new migration:

`crates/clearcut-store/migrations/0013_review_reason_parts.sql`:

```sql
-- Written by Kone Tshivhinda & Claude
-- A page read in parts can end with one of three new review reasons: `part_conflict`,
-- `part_failed`, `too_dense` (clearcut-core `ReviewReason`). This only widens one check;
-- no data changes. Run as the owner role.

alter table page_readings drop constraint page_readings_review_reason_check;
alter table page_readings add constraint page_readings_review_reason_check
    check (review_reason in (
        'top_rung_failed', 'too_long', 'refused', 'budget', 'call_cap',
        'cannot_be_sent_safely', 'request_rejected', 'no_rung_available',
        'amounts_include_vat', 'tax_not_shown',
        'part_conflict', 'part_failed', 'too_dense'));
```

Run: `cargo test -p clearcut-store every_review_reason` with `DATABASE_URL=postgres://clearcut@127.0.0.1:54329/clearcut`. Do not add `-- --exact`: the test is module-qualified (`page_readings::tests::…`), so `--exact` matches nothing and passes without running it.
Expected: PASS. `sqlx::test` applies every migration in the folder, so the widened check is what the run proves.

- [ ] **Step 8: Commit**

```bash
git add crates/clearcut-core crates/clearcut-store/migrations/0013_review_reason_parts.sql
git commit -m "feat(core): merge part readings and name the new review reasons"
```

---

### Task 3: The worker draws a band of a page

**Files:**
- Modify: `services/worker/src/clearcut_worker/render.py`
- Modify: `services/worker/src/clearcut_worker/isolation.py`
- Modify: `services/worker/src/clearcut_worker/app.py`
- Modify: `services/worker/tests/children.py`
- Test: `services/worker/tests/test_render.py`, `services/worker/tests/test_http_render.py`, `services/worker/tests/test_render_isolation.py`

**Interfaces:**
- Consumes: nothing from earlier tasks (independent service).
- Produces:
  - `EmptyCrop(InspectError)` in `render.py`.
  - `render_page(data, page, *, dpi, fmt, max_edge, top=0, bottom=1000, ...)`: the picture holds only the band between `top` and `bottom` thousandths of the page height, cropped **before** `encode_image`, so `max_edge` sharpens the band.
  - `render_isolated(..., top=0, bottom=1000, ...)`; the child's positional tuple becomes `(data, page, dpi, fmt, max_edge, top, bottom, max_pages, timeout_seconds, max_bytes)`.
  - `POST /v1/render?page=&dpi=&format=&max_edge=&top=&bottom=`; `top` and `bottom` are optional and default to `0` and `1000`. An empty band (`top >= bottom`) is `422 {"error": "the crop is empty"}`.

- [ ] **Step 1: Write the failing tests**

In `services/worker/tests/test_render.py`, import `EmptyCrop` with the other render imports and add:

```python
def test_a_crop_returns_only_the_band_between_the_marks() -> None:
    # The page is 400 x 400 at 144 dpi and the square covers y 100 to 300.
    result = render(one_page(SQUARE), dpi=144, top=200, bottom=700)
    assert (result.width, result.height) == (400, 200)
    assert pixel(result.data, 200, 100) == BLACK  # the square, inside the band
    assert pixel(result.data, 200, 10) == WHITE  # above the square, still in the band


def test_a_picture_can_be_cropped_too() -> None:
    result = render(png_of(noise(40, 30)), top=0, bottom=500)
    assert (result.width, result.height) == (40, 15)


def test_an_empty_crop_is_refused() -> None:
    with pytest.raises(EmptyCrop):
        render(one_page(SQUARE), top=500, bottom=500)
```

In `services/worker/tests/test_http_render.py`, import `EmptyCrop` from `clearcut_worker.render` and add:

```python
def test_the_crop_is_handed_to_the_renderer() -> None:
    renderer = canned()
    app = create_app(settings(), renderer=renderer)
    response = TestClient(app).post(
        RENDER,
        content=b"%PDF-1.4 bytes",
        headers=AUTH,
        params={"top": 250, "bottom": 750},
    )
    assert response.status_code == 200
    ((_, kwargs),) = renderer.calls
    assert (kwargs["top"], kwargs["bottom"]) == (250, 750)


def test_an_empty_crop_is_a_422_that_names_it() -> None:
    def renderer(*args: Any, **kwargs: Any) -> EncodedImage:
        raise EmptyCrop

    response = TestClient(create_app(settings(), renderer=renderer)).post(
        RENDER, content=b"%PDF-x", headers=AUTH, params={"top": 800, "bottom": 800}
    )
    assert response.status_code == 422
    assert response.json() == {"error": "the crop is empty"}
```

In the existing `test_the_app_hands_the_renderer_what_was_asked_for_and_the_settings`, extend the exact kwargs dict with `"top": 0, "bottom": 1000` (insert `top` and `bottom` after `max_edge`). Do the same in `test_without_parameters_the_renderer_gets_the_defaults` by asserting `(kwargs["top"], kwargs["bottom"]) == (0, 1000)`.

In `services/worker/tests/test_render_isolation.py`, add:

```python
def test_the_crop_is_passed_to_the_child_and_comes_back_cropped() -> None:
    # SQUARE covers y 100 to 300 of a 400 x 400 page at 144 dpi.
    image = render(one_page(SQUARE), top=200, bottom=700)
    assert (image.width, image.height) == (400, 200)
```

- [ ] **Step 2: Run the new tests to see them fail**

Run: `python -m pytest services/worker/tests/test_render.py services/worker/tests/test_http_render.py services/worker/tests/test_render_isolation.py -k "crop" -q`
Expected: FAIL with `ImportError: cannot import name 'EmptyCrop'` or `TypeError: render_page() got an unexpected keyword argument 'top'`.

- [ ] **Step 3: Implement the crop in `render.py`**

Add after `PageTooBig`:

```python
class EmptyCrop(InspectError):
    """The band asked for has no height."""


MIN_PER_MILLE = 0
MAX_PER_MILLE = 1000
```

Change the signature of `render_page` to:

```python
def render_page(
    data: bytes,
    page: int,
    *,
    dpi: float = DEFAULT_DPI,
    fmt: str = "png",
    max_edge: int = DEFAULT_MAX_EDGE,
    top: int = MIN_PER_MILLE,
    bottom: int = MAX_PER_MILLE,
    max_bytes: int = MAX_IMAGE_BYTES,
    max_pages: int,
    deadline: float,
    max_pixels: int = MAX_IMAGE_PIXELS,
    pdfium_open: Callable[[bytes], PdfiumDocument] = open_pdfium,
) -> EncodedImage:
```

At the top of the body, before the page check:

```python
    if not (MIN_PER_MILLE <= top <= MAX_PER_MILLE and MIN_PER_MILLE <= bottom <= MAX_PER_MILLE):
        raise ValueError("the crop must be given in thousandths of the page")
    if top >= bottom:
        raise EmptyCrop
```

Crop both branches immediately before encoding:

```python
        picture = open_image(data, max_pixels=max_pixels)
        check_time(deadline)
        picture = _crop(picture, top, bottom)
        return encode_image(picture, fmt=fmt, max_edge=max_edge, max_bytes=max_bytes)
```

and after the PDF `try/finally`:

```python
    check_time(deadline)
    picture = _crop(picture, top, bottom)
    return encode_image(picture, fmt=fmt, max_edge=max_edge, max_bytes=max_bytes)
```

Add the helper at the end of the module:

```python
def _crop(picture: object, top: int, bottom: int):  # type: ignore[no-untyped-def]
    """The horizontal band of a picture between two thousandths of its height."""
    width, height = picture.size  # type: ignore[attr-defined]
    first = height * top // 1000
    last = (height * bottom + 999) // 1000
    return picture.crop((0, first, width, last))  # type: ignore[attr-defined]
```

- [ ] **Step 4: Thread the crop through isolation and the endpoint**

In `isolation.py`, import `EmptyCrop` beside `PageTooBig` in the `from .render import ...` line and add it to `_RENDER_REFUSALS`:

```python
    "EmptyCrop": EmptyCrop,
```

Change `_render_child` to:

```python
def _render_child(
    conn: AnswerChannel,
    data: bytes,
    page: int,
    dpi: float,
    fmt: str,
    max_edge: int,
    top: int,
    bottom: int,
    max_pages: int,
    timeout_seconds: float,
    max_bytes: int,
) -> None:
```

and pass `top=top, bottom=bottom` to `render_page`. Change `render_isolated` to accept `top: int = 0, bottom: int = 1000` after `max_edge` and pass the tuple:

```python
        (data, page, dpi, fmt, max_edge, top, bottom, max_pages, timeout_seconds, max_bytes),
```

In `app.py`, import `EmptyCrop` beside `PageTooBig` and add two query parameters to the `render` endpoint after `max_edge`:

```python
        top: Annotated[int, Query(ge=0, le=1000)] = 0,
        bottom: Annotated[int, Query(ge=0, le=1000)] = 1000,
```

Pass `top=top, bottom=bottom` to `renderer`, and in `_refusal` add before the `PageTooBig` arm:

```python
    if isinstance(error, EmptyCrop):
        return WorkerError(422, {"error": "the crop is empty"})
```

In `tests/children.py`, add `top: int` and `bottom: int` after `max_edge: int` in `render_hang`, `render_crash` and `render_echo`, and update the comment above them.

- [ ] **Step 5: Run the worker tests until green**

Run: `python -m pytest services/worker/tests/test_render.py services/worker/tests/test_http_render.py services/worker/tests/test_render_isolation.py services/worker/tests/test_isolation.py -q`
Expected: PASS, including the unchanged full-page tests (a crop of `0..1000` returns the whole picture).

Then run the worker linters exactly as CI does:

```
cd services/worker && python -m ruff check . && python -m mypy src && python -m pytest -q
```

- [ ] **Step 6: Commit**

```bash
git add services/worker
git commit -m "feat(worker): draw only the band of a page that was asked for"
```

---

### Task 4: A part is masked from its own words and its own band

**Files:**
- Modify: `crates/clearcut-docproc/src/request/mod.rs` (and `request/tests.rs`)
- Modify: `crates/clearcut-docproc/src/http/mod.rs` (and `http/page_tests.rs`)
- Modify: `crates/clearcut-docproc/src/masker/mod.rs` (and `masker/tests.rs`)
- Modify: `crates/clearcut-docproc/src/lib.rs` (exports)
- Test: the three test files above

**Interfaces:**
- Consumes: `Crop` values travel to the worker as `top`/`bottom` in `/v1/render` (Task 3).
- Produces:
  - `pub struct Crop { top: u16, bottom: u16 }` with `Crop::new(top, bottom) -> Result<Crop, InvalidRequest>` (requires `top < bottom <= 1000`), `top()`, `bottom()`.
  - `RenderOptions::with_crop(self, crop: Crop) -> RenderOptions` and `crop(&self) -> Option<Crop>`.
  - `pub struct PartWindow<'a> { pub text: &'a PageText, pub byte_start: usize, pub crop: Crop }`.
  - `WorkerMasker::prepare_part(processor, image, page, window, spans) -> Result<WorkerMasker, MaskError>`, where `spans` are byte ranges of `window.text.content`.
  - Re-export `Crop` and `PartWindow` from `clearcut_docproc`.

- [ ] **Step 1: Write the failing tests**

In `crates/clearcut-docproc/src/request/tests.rs` add:

```rust
#[test]
fn a_crop_is_a_band_between_two_thousandths_of_the_page() {
    assert!(Crop::new(0, 1000).is_ok());
    assert!(Crop::new(250, 750).is_ok());
    assert!(Crop::new(500, 500).is_err(), "an empty band");
    assert!(Crop::new(600, 400).is_err(), "upside down");
    assert!(Crop::new(0, 1001).is_err(), "past the page");
    let options = RenderOptions::default().with_crop(Crop::new(250, 750).unwrap());
    assert_eq!(options.crop().map(|c| (c.top(), c.bottom())), Some((250, 750)));
    assert!(RenderOptions::default().crop().is_none());
}
```

In `crates/clearcut-docproc/src/http/page_tests.rs` add:

```rust
#[tokio::test]
async fn a_cropped_page_sends_the_band_in_the_query() {
    let s = serve(picture_script("image/png", "1000", "500", PNG)).await;
    let options = RenderOptions::default().with_crop(Crop::new(250, 750).unwrap());
    render(&s, options).await.unwrap();
    assert_eq!(
        query_of(&s.requests()[0]),
        pairs(&[
            ("page", "2"),
            ("dpi", "150"),
            ("format", "png"),
            ("max_edge", "2000"),
            ("top", "250"),
            ("bottom", "750"),
        ])
    );
}
```

Import `Crop` in that test module. Keep the existing exact-query test unchanged for an uncropped page: an uncropped render must **not** send `top` or `bottom`.

In `crates/clearcut-docproc/src/masker/tests.rs` add:

```rust
#[tokio::test]
async fn a_part_paints_only_its_own_words_in_its_own_band() {
    // Rows at y=100 (the two heading words) and y=300 (the number); the band is 300..600.
    let page = page_content(
        PageSource::TextLayer,
        "Account No:\n1234567890",
        Some(vec![
            word(0, 7, 100, 100, 300, 40),
            word(8, 11, 420, 100, 100, 40),
            word(12, 22, 540, 300, 250, 40),
        ]),
    );
    let part = PageText {
        source: TextSource::TextLayer,
        content: "1234567890".to_string(),
    };
    let window = PartWindow {
        text: &part,
        byte_start: 12,
        crop: Crop::new(300, 600).unwrap(),
    };
    let fake = FakeWorker::scripted(vec![]).with_masks(vec![Ok(painted(ImageType::Png))]);
    let masker =
        WorkerMasker::prepare_part(&fake, &image(), &page, &window, &[span(0, 10)])
            .await
            .unwrap();
    assert_eq!(fake.mask_calls()[0].boxes, [mask_box(537, 0, 256, 137)]);
    assert!(masker.mask(&image(), &part, &[span(0, 10)]).is_ok());
}

#[tokio::test]
async fn a_word_that_crosses_the_part_edge_cannot_be_painted() {
    let page = page_content(
        PageSource::TextLayer,
        "1234567890",
        Some(vec![word(0, 10, 540, 280, 250, 40)]), // y 280..320 crosses 300
    );
    let part = PageText {
        source: TextSource::TextLayer,
        content: "1234567890".to_string(),
    };
    let window = PartWindow {
        text: &part,
        byte_start: 0,
        crop: Crop::new(300, 600).unwrap(),
    };
    let fake = FakeWorker::scripted(vec![]);
    assert!(WorkerMasker::prepare_part(&fake, &image(), &page, &window, &[span(0, 10)])
        .await
        .is_err());
    assert_eq!(fake.call_count(), 0, "nothing is sent when the words cannot vouch for the picture");
}
```

Import `Crop` and `PartWindow` in that module.

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p clearcut-docproc crop part_`
Expected: FAIL to compile, `cannot find type Crop in this scope`.

- [ ] **Step 3: Add `Crop` to the request options**

In `request/mod.rs` above `RenderOptions`:

```rust
/// A horizontal band of the page: from `top` to `bottom` thousandths of its height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Crop {
    top: u16,
    bottom: u16,
}

impl Crop {
    /// A band with a height: `top < bottom`, both at most 1000.
    pub fn new(top: u16, bottom: u16) -> Result<Self, InvalidRequest> {
        if top >= bottom || bottom > 1000 {
            return Err(InvalidRequest);
        }
        Ok(Crop { top, bottom })
    }

    pub fn top(&self) -> u16 {
        self.top
    }

    pub fn bottom(&self) -> u16 {
        self.bottom
    }
}
```

Add `crop: Option<Crop>` to `RenderOptions` (set `None` in `new` and `Default`), plus:

```rust
    pub fn with_crop(mut self, crop: Crop) -> Self {
        self.crop = Some(crop);
        self
    }

    pub fn crop(&self) -> Option<Crop> {
        self.crop
    }
```

In `http/mod.rs`, after `max_edge`:

```rust
        if let Some(crop) = options.crop() {
            url.query_pairs_mut()
                .append_pair("top", &crop.top().to_string())
                .append_pair("bottom", &crop.bottom().to_string());
        }
```

In `lib.rs` exports, add `Crop` to the `request` line and `PartWindow` to the `masker` line.

- [ ] **Step 4: Add `PartWindow` and `prepare_part`**

In `masker/mod.rs`, add:

```rust
/// The slice of a page a part holds, and the band of the picture it was drawn from.
pub struct PartWindow<'a> {
    /// The part's text: exactly `page.text()[byte_start..byte_start + text.content.len()]`.
    pub text: &'a PageText,
    pub byte_start: usize,
    pub crop: Crop,
}
```

Add a private helper that maps a page word into the part, or `None` when the word lies outside the slice or the band:

```rust
fn word_in_part(word: &PageWord, window: &PartWindow<'_>) -> Option<PageWord> {
    let start = word.start.checked_sub(window.byte_start)?;
    let end = word.end.checked_sub(window.byte_start)?;
    if end > window.text.content.len()
        || !window.text.content.is_char_boundary(start)
        || !window.text.content.is_char_boundary(end)
    {
        return None;
    }
    let b = word.bounds;
    let (top, bottom) = (u32::from(window.crop.top()), u32::from(window.crop.bottom()));
    let first = u32::from(b.y);
    let last = first.checked_add(u32::from(b.h))?;
    if first < top || last > bottom {
        return None;
    }
    let height = bottom - top;
    let mapped_y = u16::try_from((first - top) * 1000 / height).ok()?;
    let mapped_bottom = u16::try_from(((last - top) * 1000 + height - 1) / height).ok()?;
    Some(PageWord {
        start,
        end,
        bounds: crate::WordBox {
            x: b.x,
            y: mapped_y,
            w: b.w,
            h: mapped_bottom.saturating_sub(mapped_y).max(1),
        },
        confidence: word.confidence,
    })
}
```

Add `prepare_part`:

```rust
    /// Has the worker paint over `spans` (byte ranges of `window.text.content`) inside the band.
    ///
    /// The page's words are mapped into the part; a word that lies outside the slice or the band
    /// is left out, so a span on it cannot be located and the mask fails (fail closed).
    pub async fn prepare_part<P: DocumentProcessor>(
        processor: &P,
        image: &PageImage,
        page: &PageContent,
        window: &PartWindow<'_>,
        spans: &[Span],
    ) -> Result<WorkerMasker, MaskError> {
        let same_reading = matches!(
            (page.source(), window.text.source),
            (PageSource::TextLayer, TextSource::TextLayer) | (PageSource::Ocr, TextSource::Ocr)
        );
        let expected = page
            .text()
            .get(window.byte_start..window.byte_start + window.text.content.len());
        if !same_reading || expected != Some(window.text.content.as_str()) {
            return Err(MaskError::Failed);
        }
        let masked = if spans.is_empty() {
            image.clone()
        } else {
            let words = page.words().ok_or(MaskError::Failed)?;
            let mapped: Vec<PageWord> = words
                .iter()
                .filter_map(|word| word_in_part(word, window))
                .collect();
            let boxes =
                boxes_for_spans(&window.text.content, &mapped, spans).map_err(|_| MaskError::Failed)?;
            let painted = processor
                .mask_image(
                    Bytes::from(image.bytes.clone()),
                    boxes,
                    ImageFormat::of(image.media),
                    MAX_EDGE,
                )
                .await
                .map_err(|_| MaskError::Failed)?;
            PageImage {
                media: painted.media(),
                bytes: painted.bytes().to_vec(),
            }
        };
        Ok(WorkerMasker {
            original: image.clone(),
            text: window.text.clone(),
            spans: spans.to_vec(),
            masked,
        })
    }
```

Import `Crop` with the other request imports at the top of `masker/mod.rs`.

- [ ] **Step 5: Run the docproc tests until green**

Run: `cargo test -p clearcut-docproc`
Expected: PASS, including the unchanged whole-page masking tests.

- [ ] **Step 6: Commit**

```bash
git add crates/clearcut-docproc
git commit -m "feat(docproc): crop a page band and mask a part from its own words"
```

---

### Task 5: The pack names the part, and the prompt knows what a part is

**Files:**
- Modify: `crates/clearcut-pack/src/input.rs`
- Modify: `crates/clearcut-pack/src/build.rs`
- Modify: `crates/clearcut-pack/src/manifest.rs`
- Modify: `crates/clearcut-pack/src/lib.rs`
- Modify: `crates/clearcut-pack/src/prompt.rs`
- Move: `crates/clearcut-pack/prompts/prompt-v3/` to `crates/clearcut-pack/prompts/prompt-v4/`
- Delete: `crates/clearcut-pack/tests/prefix-prompt-v3.txt` (replaced by the regenerated v4 golden)
- Test: `crates/clearcut-pack/tests/prompt.rs`, `crates/clearcut-pack/tests/pack.rs`, `crates/clearcut-pack/tests/route.rs` (compile fixes only)

**Interfaces:**
- Consumes: nothing (the pack only labels a part; the router supplies it in Task 7).
- Produces:
  - `pub struct PartRef { pub index: u8, pub of: u8 }` (1-based), `Serialize` and `Deserialize`.
  - `PageInput.part: Option<PartRef>`; every existing construction sets `part: None`.
  - `PackManifest.part: Option<PartRef>`.
  - The context block gains the line `part: {index} of {of} of page {page} of {of_pages}`; the instruction becomes `Read part {index} of {of} of page {page} of {of_pages} now and return only the JSON.`
  - `PROMPT_VERSION = "prompt-v4"`; `prompt_version()` is `reading-v3/prompt-v4`.

- [ ] **Step 1: Write the failing tests**

In `crates/clearcut-pack/tests/prompt.rs`:

```rust
const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/prefix-prompt-v4.txt");

#[test]
fn the_versions_are_named() {
    assert_eq!(PROMPT_VERSION, "prompt-v4");
    assert_eq!(prompt_version(), format!("{READING_SHAPE_VERSION}/prompt-v4"));
    assert_eq!(prompt_version(), "reading-v3/prompt-v4");
    assert!(GOLDEN.ends_with("prefix-prompt-v4.txt"));
}
```

Add `"only that part of the page"` to the list in `the_rules_tell_the_model_what_is_data_and_what_to_return`.

In `crates/clearcut-pack/tests/pack.rs`, import `PartRef` and add:

```rust
#[test]
fn a_part_is_named_in_the_context_and_the_instruction() {
    let mut input = page("Invoice No: INV-1");
    input.part = Some(PartRef { index: 2, of: 3 });
    let pack = build(&settings(), &input);
    let blocks = blocks(&pack.request);
    assert!(
        blocks.iter().any(|b| b.contains("part: 2 of 3 of page 2 of 5")),
        "{blocks:?}"
    );
    assert!(
        blocks.iter().any(|b| b
            .contains("Read part 2 of 3 of page 2 of 5 now and return only the JSON.")),
        "{blocks:?}"
    );
    assert_eq!(pack.manifest.part, Some(PartRef { index: 2, of: 3 }));
}
```

Change the manifest test's expected prompt version to `reading-v3/prompt-v4`.

In `crates/clearcut-pack/tests/route.rs`, add `part: None` to every `PageInput` literal (the compiler will name them).

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p clearcut-pack`
Expected: FAIL to compile (`PartRef` is missing, `PageInput` has no `part`) and `prompt-v3` assertions fail.

- [ ] **Step 3: Move the prompt and add the rule**

```bash
git mv crates/clearcut-pack/prompts/prompt-v3 crates/clearcut-pack/prompts/prompt-v4
git rm crates/clearcut-pack/tests/prefix-prompt-v3.txt
```

In `prompts/prompt-v4/rules.md`, after the paragraph that begins "If the page text and the image disagree", add:

```markdown
- If the context says this is part 1 of 3 of a page, you are shown only that part of the page: read only what is visible in it, and never guess a line that is cut off at the edge of the part.
```

Nothing else in `rules.md` or `examples.md` changes.

- [ ] **Step 4: Add `PartRef`, label the part, and bump the version**

In `input.rs`, above `PageInput`:

```rust
/// Which part of a page a request reads, when the page is too long for one answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PartRef {
    /// 1-based.
    pub index: u8,
    pub of: u8,
}
```

Add to `PageInput`:

```rust
    /// `Some` when this request reads one band of the page instead of all of it.
    pub part: Option<PartRef>,
```

In `manifest.rs`, add to `PackManifest`:

```rust
    /// Which part of the page this call read; `None` for a whole page.
    pub part: Option<PartRef>,
```

and `use crate::input::PartRef;` at the top.

In `build.rs`:

```rust
use crate::input::{..., PartRef, ...};
```

In `document_context_block`, after the `page:` line:

```rust
    if let Some(part) = page.part {
        lines.push(format!(
            "part: {} of {} of page {} of {}",
            part.index, part.of, page.page, page.of
        ));
    }
```

Replace the instruction with:

```rust
    let instruction = match page.part {
        Some(part) => format!(
            "Read part {} of {} of page {} of {} now and return only the JSON.",
            part.index, part.of, page.page, page.of
        ),
        None => format!(
            "Read page {} of {} now and return only the JSON.",
            page.page, page.of
        ),
    };
```

In the manifest literal add:

```rust
        part: page.part,
```

In `prompt.rs`:

```rust
pub const PROMPT_VERSION: &str = "prompt-v4";

const RULES: &str = include_str!("../prompts/prompt-v4/rules.md");
const EXAMPLES: &str = include_str!("../prompts/prompt-v4/examples.md");
```

In `lib.rs`, export `PartRef` with the other input types.

- [ ] **Step 5: Regenerate the golden and run the tests until green**

Run: `UPDATE_GOLDEN=1 cargo test -p clearcut-pack the_prompt_has_not_changed_by_accident`
Then: `cargo test -p clearcut-pack`
Expected: PASS. The golden now ends in `prefix-prompt-v4.txt`; no test writes a v3 file.

- [ ] **Step 6: Commit**

```bash
git add crates/clearcut-pack
git commit -m "feat(pack): name the part in the context and instruction, and move to prompt-v4"
```

---

### Task 6: The router can evaluate a view of a page, and reports part evidence

**Files:**
- Modify: `crates/clearcut-router/src/lib.rs`
- Modify: `crates/clearcut-router/src/tests.rs`
- Modify: `crates/clearcut-pipeline/src/read.rs` (only if a `PageResult` field must be named; see Step 4)

**Interfaces:**
- Consumes: `PageText` and `TextTrust` already held by `Material`.
- Produces:
  - `struct View<'a> { text: Option<&'a PageText>, trust: TextTrust, whole_document: bool }` (private).
  - `evaluate(task, &View, reading, model, confidence)` and `failure_report(task, &View, last)`; behaviour is unchanged for a whole page.
  - `PageResult.parts: Option<PartsEvidence>`, `None` until Task 7.
  - `pub struct PartsEvidence { pub planned: u8, pub read: u8, pub seams: Vec<u16>, pub seam_uncertain: bool, pub failed: Option<PartFailure> }`.
  - `pub struct PartFailure { pub part: u8, pub reason: ReviewReason }`.
  - `ReadingOrigin` derives `Serialize` and `Deserialize`.

- [ ] **Step 1: Write the failing test**

In `crates/clearcut-router/src/tests.rs`:

```rust
#[tokio::test]
async fn a_page_without_parts_reports_no_parts_evidence() {
    let provider = FakeProvider::scripted(vec![Ok(answer(&clean_reading()))]);
    let worker = worker(PAGE_TEXT, 1);
    let result = read(&provider, &worker, &trusted_page()).await;
    assert!(result.parts.is_none());
}

#[test]
fn a_reading_origin_survives_being_stored() {
    let origin = ReadingOrigin {
        rung: 1,
        attempt: 2,
        model: HAIKU.to_string(),
    };
    let stored = serde_json::to_string(&origin).unwrap();
    assert_eq!(serde_json::from_str::<ReadingOrigin>(&stored).unwrap(), origin);
}
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p clearcut-router a_page_without_parts a_reading_origin`
Expected: FAIL (`PageResult` has no `parts`, `ReadingOrigin` does not implement `Serialize`).

- [ ] **Step 3: Introduce `View` and move `parts` through the result**

Add near `Material`:

```rust
/// The text a reading is checked against, and whether that text can be believed.
struct View<'a> {
    text: Option<&'a PageText>,
    trust: TextTrust,
    /// Compare the printed totals with the lines. False for a part of a page.
    whole_document: bool,
}
```

Change `evaluate` to take `view: &View<'_>` and use `view.text`, `view.trust`, `view.whole_document`. Change `failure_report` to take `view: &View<'_>` and pass it to `evaluate`.

Add to `PageResult`:

```rust
    /// How the page was split and read, when it was read in parts.
    pub parts: Option<PartsEvidence>,
```

Add the public structs near `ReadingOrigin`:

```rust
/// How a page that was read in parts was planned and how far it got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartsEvidence {
    pub planned: u8,
    pub read: u8,
    /// The bottom of each seam, in thousandths of the page (one fewer than the parts).
    pub seams: Vec<u16>,
    /// A seam the planner could not place exactly. False in this slice.
    pub seam_uncertain: bool,
    pub failed: Option<PartFailure>,
}

/// The part that stopped a page from being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartFailure {
    /// 1-based.
    pub part: u8,
    pub reason: ReviewReason,
}
```

Derive `Serialize, Deserialize` on `ReadingOrigin`.

Add to `Run` (for Task 7) and to `finish`'s literal:

```rust
            parts: None,
```

At the three `evaluate`/`failure_report` call sites in `attempt`, `on_response` and `finish`, build the view:

```rust
        let view = View {
            text: material.text.as_ref(),
            trust: material.trust,
            whole_document: task.of.get() == 1,
        };
```

Place it before the call and pass `&view`. In `finish`, build it from `run.material` inside the `match`; do not move `run` first.

- [ ] **Step 4: Fix the remaining compile errors**

Run `cargo test -p clearcut-router a_page_without_parts a_reading_origin` and fix every construction of `PageResult`/`ReadingOrigin` the compiler names. `clearcut-pipeline` reads `PageResult` but only constructs `PageState`; it should need no change beyond the fields it already names.

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/clearcut-router
git commit -m "refactor(router): evaluate a view of the page and report part evidence"
```

---

### Task 7: The router plans, reads and merges a page's parts

**Files:**
- Create: `crates/clearcut-router/src/parts.rs`
- Modify: `crates/clearcut-router/src/lib.rs`
- Modify: `crates/clearcut-router/src/tests.rs`
- Modify: `crates/clearcut-core/src/escalation/reader.rs`
- Modify: `crates/clearcut-core/src/escalation/tests_reader.rs`
- Modify: `crates/clearcut-pipeline/src/read.rs` (one line: the injection list is now `Vec<String>`)

**Interfaces:**
- Consumes: `plan_parts`, `PartsPlan`, `Part`, `PositionedWord` (Task 1); `merge_parts` (Task 2); `Crop`, `PartWindow`, `RenderOptions::with_crop`, `WorkerMasker::prepare_part` (Task 4); `PartRef`, `PageInput.part` (Task 5); `View`, `PartsEvidence`, `PartFailure`, `ReadingOrigin` (Task 6).
- Produces:
  - Core: `PageReader::start_after(policy, document_calls_left, attempts_before) -> (PageReader, Step)` (attempt numbers continue from `attempts_before`), and `PageReader::attempts_issued() -> u32`. `start` becomes `start_after(policy, document_calls_left, 0)`.
  - `pub mod parts` in `clearcut-router`: `pub const EXTRA_CALLS_PER_PART: u32 = 3;`, `pub struct PartDone`, `pub struct PartsCarry`.
  - `Carry.parts: Option<PartsCarry>` (`#[serde(default)]`), `Interruption.parts: Option<PartsEvidence>`.
  - `PageResult.injection_suspected` becomes `Vec<String>` (it was `Vec<&'static str>`).
  - A page of at least 1 200 words in two rows is planned into parts and read part by part; a page with no plan is read whole exactly as before.

- [ ] **Step 1: Write the failing core test**

In `crates/clearcut-core/src/escalation/tests_reader.rs`, add at the end:

```rust
#[test]
fn a_part_reader_numbers_its_attempts_after_the_page_s_earlier_calls() {
    let policy = RouterPolicy::default_ladder();
    let (reader, step) = PageReader::start_after(&policy, 5, 2);
    let Step::Call(plan) = step else {
        panic!("a fresh part asks for its first call");
    };
    assert_eq!(plan.attempt, 3, "the page's earlier calls are counted");
    assert_eq!(reader.attempts_issued(), 3);
}
```

- [ ] **Step 2: Write the failing router tests**

Add to `crates/clearcut-router/src/tests.rs` (after the existing helpers, before the normal case is fine):

```rust
// ---------- a page read in parts ----------

/// The invented dense page: `words` tokens of "R1.00", forty to a row.
fn long_layer(words: usize) -> PageContent {
    let mut text = String::new();
    let mut boxes = Vec::new();
    for i in 0..words {
        if i > 0 {
            text.push(' ');
        }
        let start = text.len();
        text.push_str("R1.00");
        let end = text.len();
        let row = u32::try_from(i / 40).unwrap();
        let column = u32::try_from(i % 40).unwrap();
        boxes.push(PageWord {
            start,
            end,
            bounds: WordBox {
                x: u16::try_from(column * 14).unwrap(),
                y: u16::try_from(10 + row * 15).unwrap(),
                w: 13,
                h: 8,
            },
            confidence: None,
        });
    }
    PageContent::from_parts(PageParts {
        page: n(1),
        source: PageSource::TextLayer,
        text,
        width_pt: 595.0,
        height_pt: 842.0,
        fonts: Vec::new(),
        words: Some(boxes),
        agreement: None,
    })
    .unwrap()
}

fn long_worker(words: usize, pictures: usize) -> Arc<FakeWorker> {
    Arc::new(
        FakeWorker::scripted(vec![])
            .with_texts(vec![Ok(vec![long_layer(words)])])
            .with_renders(renders(pictures)),
    )
}

/// What one part of the dense page "says": one line whose amount is on every row.
fn part_reading() -> PageReading {
    PageReading {
        currency: ReadCurrency::Zar,
        amounts_include_vat: ReadVatBasis::No,
        invoice_number: String::new(),
        invoice_date: String::new(),
        supplier: ReadParty {
            name: "Acme (Pty) Ltd".into(),
            address: String::new(),
            vat_number: String::new(),
        },
        recipient: ReadParty::default(),
        lines: vec![ReadLine {
            description: "R1.00".into(),
            quantity: "1".into(),
            unit_price: "1.00".into(),
            line_total: "1.00".into(),
            tax: ReadTax::Standard,
        }],
        totals: ReadTotals::default(),
        carried: Default::default(),
        due: ReadDue::default(),
        credit_note: ReadCreditNote::default(),
    }
}

fn other_supplier() -> PageReading {
    let mut reading = part_reading();
    reading.supplier.name = "Other (Pty) Ltd".into();
    reading
}

#[tokio::test]
async fn a_dense_page_is_read_in_parts_and_merged() {
    let provider = FakeProvider::scripted(vec![
        answer(&part_reading()),
        answer(&part_reading()),
        answer(&part_reading()),
    ]);
    let worker = long_worker(1_203, 3);
    let result = read(&provider, &worker, &trusted_page()).await;

    assert_eq!(read_outcome(&result), (0, 1, ConfidenceBasis::TextConfirmed));
    assert_eq!(provider.call_count(), 3, "one call for each of three parts");
    let labels: Vec<String> = provider
        .requests()
        .iter()
        .map(|request| page_text_tag(request))
        .collect();
    assert!(labels[0].contains("part: 1 of 3 of page 1 of 1"), "{}", labels[0]);
    assert!(labels[1].contains("part: 2 of 3 of page 1 of 1"), "{}", labels[1]);
    assert!(labels[2].contains("part: 3 of 3 of page 1 of 1"), "{}", labels[2]);
    assert_eq!(
        result.parts,
        Some(PartsEvidence {
            planned: 3,
            read: 3,
            seams: vec![171, 321],
            seam_uncertain: false,
            failed: None,
        })
    );
    let extraction = result.extraction.as_ref().expect("the merged page");
    assert_eq!(extraction.lines.len(), 3, "the parts' lines are joined");
    assert_eq!(extraction.currency, Currency::Zar);
}

#[tokio::test]
async fn splitting_a_page_does_not_take_a_later_pages_calls() {
    // The document has one call left. Three parts are allowed three extras each.
    let provider = FakeProvider::scripted(vec![
        answer(&part_reading()),
        answer(&part_reading()),
        answer(&part_reading()),
    ]);
    let worker = long_worker(1_203, 3);
    let result = finished(
        router(&provider, &worker, config())
            .read_page(&trusted_page(), &mut budget(), 1, None)
            .await,
    );
    assert_eq!(read_outcome(&result)[0], 0);
    assert_eq!(provider.call_count(), 3);
}

#[tokio::test]
async fn a_part_that_is_refused_stops_the_page() {
    let provider = FakeProvider::scripted(vec![
        answer(&part_reading()),
        Err(ProviderError::Refused { usage: usage() }),
    ]);
    let worker = long_worker(1_203, 2);
    let result = finished(
        router(&provider, &worker, config())
            .read_page(&trusted_page(), &mut budget(), 100, None)
            .await,
    );
    assert_eq!(review_outcome(&result), (ReviewReason::PartFailed, None));
    assert_eq!(provider.call_count(), 2);
    let parts = result.parts.expect("part evidence");
    assert_eq!((parts.planned, parts.read), (3, 1));
    assert_eq!(
        parts.failed,
        Some(PartFailure {
            part: 2,
            reason: ReviewReason::Refused
        })
    );
}

#[tokio::test]
async fn a_part_still_too_long_is_too_dense() {
    let cut = || {
        Err(ProviderError::Truncated {
            usage: Usage {
                input_tokens: 6_000,
                output_tokens: 16_000,
                ..Usage::default()
            },
        })
    };
    // Part 1 reads; part 2 is cut off at its cap and again at the truncation ceiling. The page
    // is denser than any part can hold, and splitting further is not allowed: a person decides.
    let provider = FakeProvider::scripted(vec![answer(&part_reading()), cut(), cut()]);
    let worker = long_worker(1_203, 2);
    let result = finished(
        router(&provider, &worker, config())
            .read_page(&trusted_page(), &mut budget(), 100, None)
            .await,
    );
    assert_eq!(review_outcome(&result), (ReviewReason::TooDense, None));
    assert_eq!(provider.call_count(), 3);
    let parts = result.parts.expect("part evidence");
    assert_eq!((parts.planned, parts.read), (3, 1));
    assert_eq!(
        parts.failed,
        Some(PartFailure {
            part: 2,
            reason: ReviewReason::TooDense
        })
    );
}

#[tokio::test]
async fn a_part_that_cannot_be_afforded_stops_the_page_as_a_budget_stop() {
    let provider = FakeProvider::scripted(vec![]);
    let worker = long_worker(1_203, 1);
    let tiny = BudgetPolicy::new(
        Cost::from_nano_rand(1),
        Cost::from_nano_rand(1),
        Cost::from_rand_cents(10_000),
    )
    .unwrap();
    let mut budget = BudgetCheck::new(&tiny, 1, Cost::ZERO);
    let result = finished(
        router(&provider, &worker, config())
            .read_page(&trusted_page(), &mut budget, 100, None)
            .await,
    );
    assert_eq!(review_outcome(&result), (ReviewReason::Budget, None));
    assert_eq!(provider.call_count(), 0, "no call is made");
    assert_eq!(
        result.parts.and_then(|parts| parts.failed),
        Some(PartFailure {
            part: 1,
            reason: ReviewReason::Budget
        })
    );
}

#[tokio::test]
async fn two_parts_that_disagree_go_to_a_person() {
    let provider = FakeProvider::scripted(vec![
        answer(&part_reading()),
        answer(&other_supplier()),
        answer(&part_reading()),
    ]);
    let worker = long_worker(1_203, 3);
    let result = read(&provider, &worker, &trusted_page()).await;
    assert_eq!(review_outcome(&result), (ReviewReason::PartConflict, None));
    assert_eq!(provider.call_count(), 3);
    let parts = result.parts.expect("part evidence");
    assert_eq!((parts.planned, parts.read, parts.failed), (3, 3, None));
    assert!(result.reading.is_none(), "a disagreed page has no draft");
}

#[tokio::test]
async fn a_page_read_in_parts_resumes_where_it_stopped() {
    let first = FakeProvider::scripted(vec![
        answer(&part_reading()),
        Err(ProviderError::Timeout),
    ]);
    let first_worker = long_worker(1_203, 2);
    let interruption = interrupted(
        router(&first, &first_worker, config())
            .read_page(&trusted_page(), &mut budget(), 100, None)
            .await,
    );
    let carry = interruption.carry;
    let parts = carry.parts.as_ref().expect("the parts carry");
    assert_eq!((parts.index, parts.done.len(), parts.issued_before), (1, 1, 1));
    assert_eq!(carry.machine.pending().map(|plan| plan.attempt), Some(2));
    assert_eq!(carry.machine.attempts_issued(), 2);
    let stored = serde_json::to_string(&carry).unwrap();
    let carry: Carry = serde_json::from_str(&stored).expect("a checked carry");

    let second = FakeProvider::scripted(vec![answer(&part_reading()), answer(&part_reading())]);
    let second_worker = long_worker(1_203, 2);
    let result = finished(
        router(&second, &second_worker, config())
            .read_page(&trusted_page(), &mut budget(), 100, Some(carry))
            .await,
    );
    assert_eq!(read_outcome(&result), (0, 1, ConfidenceBasis::TextConfirmed));
    let labels: Vec<String> = second
        .requests()
        .iter()
        .map(|request| page_text_tag(request))
        .collect();
    assert!(labels[0].contains("part: 2 of 3"), "{}", labels[0]);
    assert!(labels[1].contains("part: 3 of 3"), "{}", labels[1]);
    let parts = result.parts.expect("part evidence");
    assert_eq!((parts.planned, parts.read), (3, 3));
}

#[tokio::test]
async fn parts_of_an_unverified_layer_are_not_painted_from() {
    let provider = FakeProvider::scripted(vec![
        answer(&part_reading()),
        answer(&part_reading()),
        answer(&part_reading()),
    ]);
    let worker = long_worker(1_203, 3);
    let result = finished(
        router(&provider, &worker, config())
            .read_page(
                &task(ReadRoute::ImageFirst, PageTextSource::TextLayer),
                &mut budget(),
                100,
                None,
            )
            .await,
    );
    assert_eq!(read_outcome(&result)[0], 0);
    assert_eq!(provider.call_count(), 3);
    assert_eq!(worker.mask_calls().len(), 0, "an unverified layer paints nothing");
    assert!(worker.text_calls()[0].words, "words are needed to plan the parts");
}

#[test]
fn a_reading_too_large_for_the_store_is_too_dense() {
    let mut reading = part_reading();
    let description = "x".repeat(500);
    reading.lines = (0..600)
        .map(|_| ReadLine {
            description: description.clone(),
            quantity: "1".into(),
            unit_price: "1.00".into(),
            line_total: "1.00".into(),
            tax: ReadTax::Standard,
        })
        .collect();
    assert!(!reading_fits_store(&reading), "600 long lines do not fit the store");
    assert!(reading_fits_store(&part_reading()));
}
```

Change the existing assertion in `text_that_could_not_be_checked_confirms_but_never_condemns` (line ~705):

```rust
    assert!(
        worker.text_calls()[0].words,
        "the words of an unverified layer are needed to plan parts and to paint them"
    );
```

- [ ] **Step 3: Run to see it fail**

Run: `cargo test -p clearcut-core a_part_reader_numbers_its_attempts`
Expected: FAIL to compile (`start_after` and `attempts_issued` do not exist).
Run: `cargo test -p clearcut-router a_dense_page`
Expected: FAIL to compile (`PartsEvidence` is there, but `PartFailure` and `reading_fits_store` are not, and no page is split).

- [ ] **Step 4: Add `start_after` and `attempts_issued` to the state machine**

In `crates/clearcut-core/src/escalation/reader.rs`, replace `start` with:

```rust
    /// Starts a page. `document_calls_left` is what the document may still spend in calls
    /// (`RouterPolicy::document_call_cap` less what earlier pages used).
    pub fn start(policy: &RouterPolicy, document_calls_left: u32) -> (PageReader, Step) {
        PageReader::start_after(policy, document_calls_left, 0)
    }

    /// Starts a part of a page whose earlier attempts are already spent. Attempt numbers keep
    /// counting from `attempts_before`, so every call of the page is numbered once.
    pub fn start_after(
        policy: &RouterPolicy,
        document_calls_left: u32,
        attempts_before: u32,
    ) -> (PageReader, Step) {
        let rungs: Vec<Planned> = policy
            .rungs()
            .iter()
            .enumerate()
            .filter(|(_, rung)| rung.enabled)
            .map(|(index, rung)| Planned {
                index,
                rung: rung.clone(),
                retry_ceiling: policy.retry_ceiling_for(&rung.model),
            })
            .collect();
        let mut reader = PageReader {
            rungs,
            ceiling: policy.answer_ceiling(),
            truncation_ceiling: policy.truncation_ceiling(),
            call_limit: policy.max_calls_per_page().min(document_calls_left),
            position: 0,
            calls_made: 0,
            issued: attempts_before,
            invalid_retried: false,
            cap_override: None,
            best: None,
            pending: None,
            finished: None,
        };
        let step = if reader.rungs.is_empty() {
            reader.review(ReviewReason::NoRungAvailable)
        } else if reader.calls_made >= reader.call_limit {
            reader.review(ReviewReason::CallCap)
        } else {
            reader.issue(CallKind::First)
        };
        (reader, step)
    }
```

Add beside `calls_made`:

```rust
    /// How many calls have been handed out on this page, across every part of it.
    pub fn attempts_issued(&self) -> u32 {
        self.issued
    }
```

- [ ] **Step 5: Add the parts carry**

Create `crates/clearcut-router/src/parts.rs`:

```rust
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
```

- [ ] **Step 6: Register the module and the carry fields**

In `crates/clearcut-router/src/lib.rs`, above `mod estimate;`:

```rust
pub mod parts;
```

Add to the `clearcut_core::escalation` import: `merge_parts`, `plan_parts`, `Part`, `PartsPlan`, `PositionedWord`.
Add `PartWindow` and `Crop` to the `clearcut_docproc` import, `PartRef` to the `clearcut_pack` import, and read the carry types where used:

```rust
use parts::{PartDone, PartsCarry, EXTRA_CALLS_PER_PART};
```

Add to `Carry`:

```rust
    /// The plan and the parts done, when the page is read in parts; `None` for a whole page.
    #[serde(default)]
    pub parts: Option<PartsCarry>,
```

and to `UncheckedCarry`:

```rust
    #[serde(default)]
    parts: Option<PartsCarry>,
```

and to the `Ok(Carry { .. })` in its `TryFrom`:

```rust
            parts: raw.parts,
```

Add to `Interruption`:

```rust
    /// How the page was split and how far it got, when it was read in parts.
    pub parts: Option<PartsEvidence>,
```

Change `PageResult.injection_suspected` to `Vec<String>` and its doc stays as it is.

Change `Run` to:

```rust
struct Run {
    machine: PageReader,
    best: Option<Kept>,
    last: Option<Kept>,
    material: Option<Material>,
    /// The part being read, when the page is read in parts.
    part: Option<CurrentPart>,
    /// The plan and the parts done, when the page is read in parts.
    parts: Option<PartsCarry>,
    pictures: Vec<(Picture, Option<Crop>, RenderedPage)>,
    calls: Vec<CallRecord>,
    spent: Cost,
    unavailable: Vec<String>,
    budget_stop: Option<BudgetStop>,
    injection: Vec<String>,
}
```

Replace `note_injection` with:

```rust
    fn note_injection(&mut self, kinds: &[&'static str]) {
        self.note_names(&kinds.iter().map(|kind| (*kind).to_string()).collect::<Vec<_>>());
    }

    fn note_names(&mut self, names: &[String]) {
        for name in names {
            if !self.injection.contains(name) {
                self.injection.push(name.clone());
            }
        }
    }
```

Add beside `Material`:

```rust
/// The part being read: its slice of the page's text and its band of the picture.
#[derive(Clone)]
struct CurrentPart {
    /// 1-based.
    index: u8,
    of: u8,
    text: PageText,
    /// Where `text.content` begins in the page's text, for the painter's words.
    byte_start: usize,
    crop: Crop,
}
```

Add below `MAX_RUNG`:

```rust
/// The most JSON a merged reading may take; the store's reading column holds at most 256 KB.
const MAX_STORED_READING_BYTES: usize = 240_000;

/// Whether a merged reading fits the store's column.
fn reading_fits_store(reading: &PageReading) -> bool {
    serde_json::to_vec(reading).is_ok_and(|json| json.len() <= MAX_STORED_READING_BYTES)
}
```

- [ ] **Step 7: Restructure `read_page` and add the part loop**

Replace the whole `read_page` function with:

```rust
    /// Reads one page, escalating up the ladder on evidence, or resumes a page that was
    /// interrupted (`resume` is the [`Carry`] the interruption gave).
    ///
    /// A page dense enough to need it is planned into parts from its own words; each part is
    /// read by its own state machine (attempt numbers stay page-global) and the readings are
    /// merged. A page with no plan is read whole, exactly as before.
    ///
    /// `budget` is the document's spending so far and is updated with every call's real cost.
    /// `document_calls_left` is what the document may still spend in calls
    /// (`RouterPolicy::document_call_cap` less what earlier pages used).
    pub async fn read_page(
        &self,
        task: &PageTask,
        budget: &mut BudgetCheck,
        document_calls_left: u32,
        resume: Option<Carry>,
    ) -> PageRun {
        let (machine, mut step, best, last, parts) = match resume {
            Some(carry) => {
                let step = match (carry.machine.pending(), carry.machine.outcome()) {
                    (Some(plan), _) => Step::Call(plan.clone()),
                    (None, Some(done)) => Step::Done(done.clone()),
                    // Not reachable for a checked machine; if it ever were, a person looks.
                    (None, None) => Step::Done(PageOutcome::NeedsReview {
                        reason: ReviewReason::RequestRejected,
                        draft: None,
                    }),
                };
                (carry.machine, step, carry.best, carry.last, carry.parts)
            }
            None => {
                let (machine, step) = PageReader::start(&self.config.policy, document_calls_left);
                (machine, step, None, None, None)
            }
        };
        let mut run = Run {
            machine,
            best,
            last,
            material: None,
            part: None,
            parts,
            pictures: Vec::new(),
            calls: Vec::new(),
            spent: Cost::ZERO,
            unavailable: Vec::new(),
            budget_stop: None,
            injection: Vec::new(),
        };
        let injected = run
            .parts
            .as_ref()
            .map(|parts| parts.injection.clone())
            .unwrap_or_default();
        run.note_names(&injected);

        // The page's text chooses its parts and tells the painter where the words are. It is
        // fetched before the first call so a worker failure interrupts the page before any
        // call is made, and a resumed part page has it before its next call.
        if (run.parts.is_some() || run.machine.calls_made() == 0) && run.material.is_none() {
            match self.fetch_material(task).await {
                Ok(material) => run.material = Some(material),
                Err(trouble) => {
                    return PageRun::Interrupted(Box::new(run.into_interruption(trouble)));
                }
            }
        }
        if run.parts.is_none() && run.machine.calls_made() == 0 {
            if let Some(plan) = plan_page_parts(run.material.as_ref()) {
                run.parts = Some(PartsCarry {
                    plan,
                    index: 0,
                    done: Vec::new(),
                    issued_before: 0,
                    injection: Vec::new(),
                });
                match self.start_next_part(&mut run, document_calls_left) {
                    Ok(first) => step = first,
                    Err(trouble) => {
                        return PageRun::Interrupted(Box::new(run.into_interruption(trouble)));
                    }
                }
            }
        } else if run.parts.is_some() && run.part.is_none() {
            // A resumed part page carries on with the call it had already planned; only the
            // part's words and band have to be remembered again.
            if let Err(trouble) = self.load_part(&mut run) {
                return PageRun::Interrupted(Box::new(run.into_interruption(trouble)));
            }
        }

        loop {
            if run.parts.is_some() {
                let done = match self.read_part(&mut run, task, budget, step).await {
                    Ok(done) => done,
                    Err(trouble) => {
                        return PageRun::Interrupted(Box::new(run.into_interruption(trouble)));
                    }
                };
                let failed = match &done.outcome {
                    PageOutcome::Read { .. } => None,
                    other => Some(part_review_reason(other)),
                };
                let planned = run.parts.as_ref().map_or(0, |parts| parts.plan.parts().len());
                record_part(&mut run, done);
                if let Some(reason) = failed {
                    let outcome = PageOutcome::NeedsReview {
                        reason,
                        draft: None,
                    };
                    return PageRun::Finished(Box::new(self.finish_parts(run, task, outcome)));
                }
                if run.parts.as_ref().map_or(0, |parts| parts.done.len()) == planned {
                    let outcome = self.merge_parts_outcome(&mut run, task);
                    return PageRun::Finished(Box::new(self.finish_parts(run, task, outcome)));
                }
                match self.start_next_part(&mut run, document_calls_left) {
                    Ok(next) => step = next,
                    Err(trouble) => {
                        return PageRun::Interrupted(Box::new(run.into_interruption(trouble)));
                    }
                }
                continue;
            }

            let plan = match step {
                Step::Done(outcome) => {
                    return PageRun::Finished(Box::new(self.finish(run, task, outcome)));
                }
                Step::Call(plan) => plan,
            };
            match self.attempt(&mut run, task, &plan, budget).await {
                Ok(attempted) => {
                    step = run.machine.on_outcome(attempted.outcome);
                    if let Some(kept) = attempted.kept {
                        if run.machine.best_attempt() == Some(kept.attempt) {
                            run.best = Some(kept.clone());
                        }
                        run.last = Some(kept);
                    }
                }
                Err(trouble) => {
                    return PageRun::Interrupted(Box::new(run.into_interruption(trouble)));
                }
            }
        }
    }
```

Then add these methods after `read_page`:

```rust
    /// Remembers the part to read next: its slice of the text and its band of the picture.
    fn load_part(&self, run: &mut Run) -> Result<(), Trouble> {
        let part = {
            let parts = run
                .parts
                .as_ref()
                .ok_or_else(|| Trouble::worker_said("a part was loaded without a plan"))?;
            let index = parts.index + 1;
            let index = u8::try_from(index)
                .map_err(|_| Trouble::worker_said("a page has more parts than a plan can hold"))?;
            parts
                .plan
                .part(index)
                .ok_or_else(|| Trouble::worker_said("a part is not in the plan"))?
                .clone()
        };
        let material = run
            .material
            .as_ref()
            .ok_or_else(|| Trouble::worker_said("a part was loaded before its page was read"))?;
        let text = material
            .text
            .as_ref()
            .ok_or_else(|| Trouble::worker_said("a page without text cannot be split"))?;
        let content = text
            .content
            .get(part.byte_start()..part.byte_end())
            .ok_or_else(|| Trouble::worker_said("a part's text is not on its page"))?;
        let crop = Crop::new(part.top(), part.bottom())
            .map_err(|_| Trouble::worker_said("a part's band is outside its page"))?;
        run.part = Some(CurrentPart {
            index: part.index(),
            of: part.of(),
            text: PageText {
                source: text.source,
                content: content.to_string(),
            },
            byte_start: part.byte_start(),
            crop,
        });
        Ok(())
    }

    /// Starts the next part's state machine, with the calls made so far counted.
    fn start_next_part(&self, run: &mut Run, document_calls_left: u32) -> Result<Step, Trouble> {
        let (planned, issued_before) = {
            let parts = run
                .parts
                .as_ref()
                .ok_or_else(|| Trouble::worker_said("a part was started without a plan"))?;
            (parts.plan.parts().len(), parts.issued_before)
        };
        self.load_part(run)?;
        let left = part_allowance(document_calls_left, planned).saturating_sub(issued_before);
        let (machine, step) = PageReader::start_after(&self.config.policy, left, issued_before);
        run.machine = machine;
        run.best = None;
        run.last = None;
        Ok(step)
    }

    /// Reads one part to its end. The part is done when its own machine has nothing to do.
    async fn read_part(
        &self,
        run: &mut Run,
        task: &PageTask,
        budget: &mut BudgetCheck,
        mut step: Step,
    ) -> Result<PartDone, Trouble> {
        loop {
            let plan = match step {
                Step::Done(outcome) => {
                    let part = run
                        .part
                        .as_ref()
                        .ok_or_else(|| Trouble::worker_said("a part ended without its words"))?;
                    let index = part.index;
                    let issued = run.machine.attempts_issued().saturating_sub(
                        run.parts
                            .as_ref()
                            .map_or(0, |parts| parts.issued_before),
                    );
                    return Ok(PartDone {
                        index,
                        outcome,
                        reading: run.last.as_ref().map(|kept| kept.reading.clone()),
                        reading_from: run.last.as_ref().map(|kept| ReadingOrigin {
                            rung: kept.rung,
                            attempt: kept.attempt,
                            model: kept.model.clone(),
                        }),
                        issued,
                    });
                }
                Step::Call(plan) => plan,
            };
            let attempted = self.attempt(run, task, &plan, budget).await?;
            step = run.machine.on_outcome(attempted.outcome);
            if let Some(kept) = attempted.kept {
                if run.machine.best_attempt() == Some(kept.attempt) {
                    run.best = Some(kept.clone());
                }
                run.last = Some(kept);
            }
        }
    }

    /// Turns every part's reading into one page reading, and assesses it against the whole
    /// page's text: a conflict, a reading too large for the store, or a verdict ends the page.
    fn merge_parts_outcome(&self, run: &mut Run, task: &PageTask) -> PageOutcome {
        let (outcome, kept) = {
            let Some(parts) = run.parts.as_ref() else {
                return PageOutcome::NeedsReview {
                    reason: ReviewReason::RequestRejected,
                    draft: None,
                };
            };
            let mut readings = Vec::with_capacity(parts.done.len());
            let mut origins = Vec::with_capacity(parts.done.len());
            for done in &parts.done {
                let (Some(origin), Some(value)) = (&done.reading_from, &done.reading) else {
                    return PageOutcome::NeedsReview {
                        reason: ReviewReason::PartFailed,
                        draft: None,
                    };
                };
                let Ok(stored) = PageReading::from_stored_value(value.clone()) else {
                    return PageOutcome::NeedsReview {
                        reason: ReviewReason::PartFailed,
                        draft: None,
                    };
                };
                readings.push(stored.reading);
                origins.push(origin.clone());
            }
            let Ok(merged) = merge_parts(&readings) else {
                return PageOutcome::NeedsReview {
                    reason: ReviewReason::PartFailed,
                    draft: None,
                };
            };
            if !merged.conflicts.is_empty() {
                return PageOutcome::NeedsReview {
                    reason: ReviewReason::PartConflict,
                    draft: None,
                };
            }
            if !reading_fits_store(&merged.reading) {
                return PageOutcome::NeedsReview {
                    reason: ReviewReason::TooDense,
                    draft: None,
                };
            }
            let basis = parts
                .done
                .iter()
                .filter_map(|done| match &done.outcome {
                    PageOutcome::Read { confidence, .. } => Some(*confidence),
                    _ => None,
                })
                .min_by_key(|basis| basis.per_mille())
                .unwrap_or(ConfidenceBasis::Unresolved);
            let origin = parts
                .done
                .iter()
                .zip(origins.iter())
                .filter(|(done, _)| matches!(done.outcome, PageOutcome::Read { .. }))
                .min_by_key(|(done, _)| match &done.outcome {
                    PageOutcome::Read { confidence, .. } => confidence.per_mille(),
                    _ => u16::MAX,
                })
                .map(|(_, origin)| origin.clone());
            let Some(origin) = origin else {
                return PageOutcome::NeedsReview {
                    reason: ReviewReason::PartFailed,
                    draft: None,
                };
            };
            let Some(material) = run.material.as_ref() else {
                return PageOutcome::NeedsReview {
                    reason: ReviewReason::RequestRejected,
                    draft: None,
                };
            };
            let view = View {
                text: material.text.as_ref(),
                trust: material.trust,
                whole_document: task.of.get() == 1,
            };
            let assessment = match evaluate(
                task,
                &view,
                &merged.reading,
                &origin.model,
                ConfidenceBasis::Unresolved,
            ) {
                Ok(evaluation) => evaluation.assessment,
                Err(Unevaluable) => {
                    return PageOutcome::NeedsReview {
                        reason: ReviewReason::TopRungFailed,
                        draft: None,
                    };
                }
            };
            let kept = Kept {
                attempt: origin.attempt,
                rung: origin.rung,
                model: origin.model.clone(),
                reading: serde_json::to_value(&merged.reading).unwrap_or(Value::Null),
            };
            let outcome = match assessment.verdict {
                Verdict::Accept => PageOutcome::Read {
                    rung: origin.rung,
                    attempt: origin.attempt,
                    confidence: basis,
                    notes: assessment.notes,
                },
                Verdict::Review(reason) => PageOutcome::NeedsReview {
                    reason,
                    draft: Some(origin.attempt),
                },
                Verdict::Escalate(_) => PageOutcome::NeedsReview {
                    reason: ReviewReason::TopRungFailed,
                    draft: Some(origin.attempt),
                },
            };
            (outcome, Some(kept))
        };
        if let Some(kept) = kept {
            run.best = Some(kept.clone());
            run.last = Some(kept);
        }
        outcome
    }

    /// Finishes a page read in parts, adding how it was split.
    fn finish_parts(&self, run: Run, task: &PageTask, outcome: PageOutcome) -> PageResult {
        let evidence = evidence_of(run.parts.as_ref());
        let mut result = self.finish(run, task, outcome);
        result.parts = evidence;
        result
    }
```

Add the free functions after the `impl` block's closing brace (before the final `}` of the file there is only one impl; place them after it):

```rust
/// The writes a stored page carries on after its first part.
fn part_allowance(document_calls_left: u32, planned: usize) -> u32 {
    let parts = u32::try_from(planned).unwrap_or(u32::MAX);
    document_calls_left
        .saturating_add(EXTRA_CALLS_PER_PART.saturating_mul(parts.saturating_sub(1)))
}

/// The fixed reason a part's failure gives the page. A budget or a call cap is the page's too;
/// a part still cut off at the ceiling makes the page too dense, since splitting further is not
/// allowed; anything else is a part that could not be read.
fn part_review_reason(outcome: &PageOutcome) -> ReviewReason {
    match outcome {
        PageOutcome::NeedsReview {
            reason:
                reason @ (ReviewReason::Budget
                | ReviewReason::CallCap
                | ReviewReason::CannotBeSentSafely),
            ..
        } => *reason,
        PageOutcome::NeedsReview {
            reason: ReviewReason::TooLong,
            ..
        } => ReviewReason::TooDense,
        _ => ReviewReason::PartFailed,
    }
}

/// Splits the page from its words and text; `None` when it is not dense enough or the words
/// do not allow it.
fn plan_page_parts(material: Option<&Material>) -> Option<PartsPlan> {
    let material = material?;
    let text = material.text.as_ref()?;
    let words = material.content.as_ref()?.words()?;
    let positioned: Vec<PositionedWord> = words
        .iter()
        .map(|word| PositionedWord {
            start: word.start,
            end: word.end,
            x: word.bounds.x,
            top: word.bounds.y,
            bottom: word.bounds.y.saturating_add(word.bounds.h),
        })
        .collect();
    plan_parts(&positioned, text.content.len())
}

/// What a reading is checked against: a part's own text, or the whole page's.
fn view_of<'a>(
    task: &PageTask,
    material: &'a Material,
    part: Option<&'a CurrentPart>,
) -> View<'a> {
    match part {
        Some(part) => View {
            text: Some(&part.text),
            trust: material.trust,
            whole_document: false,
        },
        None => View {
            text: material.text.as_ref(),
            trust: material.trust,
            whole_document: task.of.get() == 1,
        },
    }
}

/// Records a finished part and clears the part's own readings for the next one.
fn record_part(run: &mut Run, done: PartDone) {
    let issued = run.machine.attempts_issued();
    if let Some(parts) = run.parts.as_mut() {
        parts.done.push(done);
        parts.index = u32::try_from(parts.done.len()).unwrap_or(u32::MAX);
        parts.issued_before = issued;
        parts.injection = run.injection.clone();
    }
    run.part = None;
    run.best = None;
    run.last = None;
}

/// How a page was split and how far it got. Counts, seams and a fixed reason only.
fn evidence_of(parts: Option<&PartsCarry>) -> Option<PartsEvidence> {
    let parts = parts?;
    let plan = &parts.plan;
    let read = parts
        .done
        .iter()
        .filter(|done| matches!(done.outcome, PageOutcome::Read { .. }))
        .count();
    Some(PartsEvidence {
        planned: u8::try_from(plan.parts().len()).unwrap_or(u8::MAX),
        read: u8::try_from(read).unwrap_or(u8::MAX),
        seams: plan
            .parts()
            .split_last()
            .map(|(_, rest)| rest.iter().map(Part::bottom).collect())
            .unwrap_or_default(),
        seam_uncertain: false,
        failed: parts
            .done
            .last()
            .filter(|done| !matches!(done.outcome, PageOutcome::Read { .. }))
            .map(|done| PartFailure {
                part: done.index,
                reason: part_review_reason(&done.outcome),
            }),
    })
}

/// The interruption a run gives when the service fails.
impl Run {
    fn into_interruption(mut self, trouble: Trouble) -> Interruption {
        if let Some(parts) = self.parts.as_mut() {
            parts.injection = self.injection.clone();
        }
        let evidence = evidence_of(self.parts.as_ref());
        let Run {
            machine,
            best,
            last,
            parts,
            calls,
            spent,
            unavailable,
            ..
        } = self;
        Interruption {
            failure: trouble.failure,
            message: trouble.message,
            retry_after: trouble.retry_after,
            key_rejected: trouble.key_rejected,
            carry: Carry {
                machine,
                best,
                last,
                parts,
            },
            calls,
            spent,
            unavailable_models: unavailable,
            parts: evidence,
        }
    }
}
```

There is already an `impl Run` block (with `note_injection` and `record_unsent`); add `into_interruption` inside that block instead of a second `impl`, and do not repeat `impl Run`.

- [ ] **Step 8: Make the call report the part's own view**

Replace the whole `attempt` function with:

```rust
    /// Makes one planned call and says what came of it.
    async fn attempt(
        &self,
        run: &mut Run,
        task: &PageTask,
        plan: &CallPlan,
        budget: &mut BudgetCheck,
    ) -> Result<Attempted, Trouble> {
        if run.material.is_none() {
            run.material = Some(self.fetch_material(task).await?);
        }
        let part = run.part.clone();
        let rendered = self
            .picture(run, task, plan.picture, part.as_ref().map(|part| part.crop))
            .await?;
        let image = PageImage {
            media: rendered.media(),
            bytes: rendered.bytes().to_vec(),
        };

        // Personal data in the picture is painted over by the worker, from the redaction that
        // the pack will compute for the same text. A text that cannot vouch for the picture
        // (an unverified layer) is not painted from, and the pack records the picture as
        // unchecked. A painter that fails is a masker that says so, and the policy decides.
        let painter = match (run.material.as_ref(), part.as_ref()) {
            (
                Some(Material {
                    text: Some(_),
                    content: Some(content),
                    ..
                }),
                Some(part),
            ) => {
                let spans = redact::redact(&part.text.content).spans;
                let window = PartWindow {
                    text: &part.text,
                    byte_start: part.byte_start,
                    crop: part.crop,
                };
                WorkerMasker::prepare_part(&*self.worker, &image, content, &window, &spans)
                    .await
                    .ok()
            }
            (
                Some(Material {
                    text: Some(text),
                    content: Some(content),
                    ..
                }),
                None,
            ) => {
                // (The painter itself refuses an unverified layer, without asking the worker.)
                let spans = redact::redact(&text.content).spans;
                WorkerMasker::prepare(&*self.worker, &image, content, text, &spans)
                    .await
                    .ok()
            }
            _ => None,
        };
        let no_painter = NoMasker;
        let masker: &dyn ImageMasker = match &painter {
            Some(painter) => painter,
            None => &no_painter,
        };

        let Some(material) = run.material.as_ref() else {
            return Err(Trouble::worker_said("the page's material was not fetched"));
        };
        let settings = PackSettings {
            model: plan.model.clone(),
            call: CallContext {
                tenant: task.tenant,
                job: task.job,
                document: Some(task.document_uuid),
                page: Some(task.page.get()),
                purpose: Purpose::Extraction,
            },
            effort: effort_of(plan.effort),
            max_output_tokens: plan.max_output_tokens,
            cache_ttl: self.config.cache_ttl,
            output_schema: self.config.output_schema.clone(),
            tenant_rules: self.config.tenant_rules.clone(),
            redaction: self.config.redaction,
        };
        let input = PageInput {
            page: task.page,
            of: task.of,
            image: Some(image),
            text: match &part {
                Some(part) => Some(part.text.clone()),
                None => material.text.clone(),
            },
            part: part
                .as_ref()
                .map(|part| PartRef {
                    index: part.index,
                    of: part.of,
                }),
        };
        let view = view_of(task, material, part.as_ref());
        let report = failure_report(task, &view, run.last.as_ref());
        let pack = match build_pack(&settings, &input, &task.context, report.as_ref(), masker) {
            Ok(pack) => pack,
            Err(PackError::CannotBeSentSafely(_)) => {
                run.record_unsent(plan, None, CallLabel::NotSent(ReviewReason::CannotBeSentSafely));
                return Attempted::just(Outcome::CannotBeSentSafely);
            }
            Err(_) => {
                run.record_unsent(plan, None, CallLabel::Rejected);
                return Attempted::just(Outcome::Rejected);
            }
        };
        let request = pack.request;
        let manifest = pack.manifest;
        run.note_injection(&manifest.injection_suspected);

        // The worst this call could cost must fit what is left, or it is not made.
        let estimate = call_estimate(&Sizes {
            prefix_bytes: manifest.prefix_bytes,
            page_text_bytes: uncached_text_bytes(&manifest.sections),
            schema_bytes: serde_json::to_string(&self.config.output_schema)
                .unwrap_or_default()
                .len(),
            width: rendered.width(),
            height: rendered.height(),
            max_output_tokens: plan.max_output_tokens,
        });
        // A model whose window cannot hold the page plus its largest answer is not asked: the
        // provider would refuse the call, and a stronger model has more room.
        if let Some(limit) = self.config.model_input_limits.get(&plan.model).copied() {
            let wanted = estimate
                .prefix_tokens
                .saturating_add(estimate.page_input_tokens)
                .saturating_add(u64::from(estimate.max_output_tokens));
            if wanted > limit {
                run.record_unsent(plan, Some(manifest), CallLabel::NotSent(ReviewReason::TooLong));
                return Attempted::just(Outcome::InputTooLarge);
            }
        }
        let worst = match self
            .config
            .prices
            .worst_case(&plan.model, &estimate, self.config.rate)
        {
            Ok(worst) => worst,
            Err(_) => {
                run.budget_stop = Some(BudgetStop::NoPrice);
                run.record_unsent(plan, Some(manifest), CallLabel::NotSent(ReviewReason::Budget));
                return Attempted::just(Outcome::BudgetRefused);
            }
        };
        if let Err(scope) = budget.check(worst) {
            run.budget_stop = Some(BudgetStop::Scope(scope));
            run.record_unsent(plan, Some(manifest), CallLabel::NotSent(ReviewReason::Budget));
            return Attempted::just(Outcome::BudgetRefused);
        }

        let sent = Sent {
            manifest,
            worst,
            elapsed: Duration::ZERO,
        };
        let started = Instant::now();
        let result = self.provider.complete(request).await;
        let sent = Sent {
            elapsed: started.elapsed(),
            ..sent
        };

        match result {
            Ok(response) => self.on_response(run, task, plan, budget, sent, response),
            Err(error) => self.on_error(run, plan, budget, sent, error),
        }
    }
```

In `on_response`, replace the material/view block with:

```rust
        let Some(material) = run.material.as_ref() else {
            return Err(Trouble::worker_said("the page's material was not fetched"));
        };
        let view = view_of(task, material, run.part.as_ref());
        let evaluation = match evaluate(task, &view, &reading, &model, ConfidenceBasis::Unresolved) {
            Ok(evaluation) => evaluation,
            Err(Unevaluable) => {
                record.label = CallLabel::Escalated(EscalationReason::ReadingProblems);
                run.calls.push(record);
                return Attempted::just(Outcome::NotConverted);
            }
        };
```

Replace `picture` with:

```rust
    /// The picture of the page, or of a part of it, at the size the rung wants, drawn once per
    /// size and band.
    async fn picture(
        &self,
        run: &mut Run,
        task: &PageTask,
        picture: Picture,
        crop: Option<Crop>,
    ) -> Result<RenderedPage, Trouble> {
        if let Some((_, _, rendered)) = run
            .pictures
            .iter()
            .find(|(drawn, drawn_crop, _)| *drawn == picture && *drawn_crop == crop)
        {
            return Ok(rendered.clone());
        }
        let mut options =
            RenderOptions::new(picture.dpi, ImageFormat::Png, picture.max_edge).map_err(|_| Trouble {
                failure: FailureKind::NeedsReview,
                message: "a rung asks for a picture size the worker does not draw".to_string(),
                retry_after: None,
                key_rejected: false,
            })?;
        if let Some(crop) = crop {
            options = options.with_crop(crop);
        }
        let rendered = self
            .worker
            .render_page(task.file.clone(), task.page, options)
            .await
            .map_err(Trouble::worker)?;
        run.pictures.push((picture, crop, rendered.clone()));
        Ok(rendered)
    }
```

In `fetch_material`, change the `ImageFirst` arm to ask for words:

```rust
            // Only a hint: its words would not be painted from, but a part needs them to be
            // planned and to be located in the picture.
            (ReadRoute::ImageFirst, PageTextSource::TextLayer) => {
                (Some(self.layer(task, true).await?), TextTrust::Unverified)
            }
```

- [ ] **Step 9: Fix the pipeline's injection list**

In `crates/clearcut-pipeline/src/read.rs`, in `save_finished`:

```rust
            injection_suspected: result.injection_suspected.clone(),
```

- [ ] **Step 10: Run the tests until green**

Run: `cargo test -p clearcut-core a_part_reader_numbers_its_attempts`
Then: `cargo test -p clearcut-router`
Then: `cargo test -p clearcut-pipeline`
Expected: PASS. Every earlier router test still passes, including the whole-page ones.

- [ ] **Step 11: Commit**

```bash
git add crates/clearcut-core crates/clearcut-router crates/clearcut-pipeline
git commit -m "feat(router): plan, read and merge a dense page's parts"
```

---

### Task 8: The pipeline records the split and widens the page's allowance

**Files:**
- Modify: `crates/clearcut-pipeline/src/read.rs`
- Modify: `crates/clearcut-pipeline/src/tests/read.rs`
- Modify: `packages/sdk/src/types.ts`

**Interfaces:**
- Consumes: `PartsEvidence` and `PartFailure` (Task 6), `Interruption.parts` and `PageResult.parts` (Task 7), `EXTRA_CALLS_PER_PART` (Task 7).
- Produces:
  - `page_notes(outcome, text_dropped_bytes, parts: Option<&PartsEvidence>) -> Value`.
  - `parts_notes(parts: &PartsEvidence) -> Vec<Value>` and `part_extras(notes: &Value) -> u32`.
  - `calls_left(tenant, document, pages, models, extra: u32)`.

- [ ] **Step 1: Write the failing tests**

In `crates/clearcut-pipeline/src/tests/read.rs`, replace the import lines above the notes tests:

```rust
use crate::read::{page_notes, part_extras, text_dropped_pages};
use clearcut_core::escalation::{PageOutcome, ReviewReason};
use clearcut_router::{PartFailure, PartsEvidence};
```

Update the two existing calls in `a_page_read_from_its_picture_says_why_in_its_notes` to pass no parts:

```rust
        page_notes(&outcome, 200_001, None),
    );
    assert_eq!(page_notes(&outcome, 0, None), serde_json::json!([]));
```

Add two tests after it:

```rust
#[test]
fn a_page_read_in_parts_says_how_it_was_split() {
    let parts = PartsEvidence {
        planned: 3,
        read: 2,
        seams: vec![171, 321],
        seam_uncertain: false,
        failed: Some(PartFailure {
            part: 3,
            reason: ReviewReason::PartFailed,
        }),
    };
    let outcome = PageOutcome::Read {
        rung: 0,
        attempt: 1,
        confidence: ConfidenceBasis::Unresolved,
        notes: Vec::new(),
    };
    assert_eq!(
        page_notes(&outcome, 0, Some(&parts)),
        serde_json::json!([
            { "type": "parts", "planned": 3, "read": 2, "seams": [171, 321], "seam_uncertain": false },
            { "type": "part_failed", "part": 3, "reason": "part_failed" }
        ])
    );
}

#[test]
fn only_a_split_page_widens_the_allowance() {
    let split = serde_json::json!([{ "type": "parts", "planned": 3, "read": 3 }]);
    assert_eq!(part_extras(&split), 6);
    assert_eq!(
        part_extras(&serde_json::json!([{ "type": "text_dropped", "bytes": 1 }])),
        0
    );
    assert_eq!(part_extras(&serde_json::json!([])), 0);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p clearcut-pipeline a_page_read_in_parts_says_how_it_was_split`
Expected: FAIL to compile (`page_notes` takes two arguments, `part_extras` does not exist).

- [ ] **Step 3: Write the notes helpers**

In `crates/clearcut-pipeline/src/read.rs`, replace `page_notes` with the three functions:

```rust
/// The notes a page carries: what the router said, what it dropped, and how it was split.
pub(crate) fn page_notes(
    outcome: &PageOutcome,
    text_dropped_bytes: usize,
    parts: Option<&PartsEvidence>,
) -> Value {
    let mut notes = match outcome {
        PageOutcome::Read { notes, .. } => serde_json::to_value(notes).unwrap_or_else(|_| json!([])),
        PageOutcome::NeedsReview { .. } => json!([]),
    };
    if text_dropped_bytes > 0 {
        if let Some(list) = notes.as_array_mut() {
            list.push(json!({ "type": "text_dropped", "bytes": text_dropped_bytes }));
        }
    }
    if let Some(parts) = parts {
        if let Some(list) = notes.as_array_mut() {
            list.extend(parts_notes(parts));
        }
    }
    notes
}

/// What a split page says about itself: the plan, the seams and a part that failed.
pub(crate) fn parts_notes(parts: &PartsEvidence) -> Vec<Value> {
    let mut notes = vec![json!({
        "type": "parts",
        "planned": parts.planned,
        "read": parts.read,
        "seams": parts.seams,
        "seam_uncertain": parts.seam_uncertain,
    })];
    if let Some(failed) = &parts.failed {
        notes.push(json!({
            "type": "part_failed",
            "part": failed.part,
            "reason": failed.reason.as_str(),
        }));
    }
    notes
}

/// How many calls a page needed because it was split, from its stored notes.
pub(crate) fn part_extras(notes: &Value) -> u32 {
    let planned = notes
        .as_array()
        .and_then(|notes| {
            notes
                .iter()
                .find(|note| note.get("type").and_then(Value::as_str) == Some("parts"))
        })
        .and_then(|note| note.get("planned"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let planned = u32::try_from(planned).unwrap_or(u32::MAX);
    planned
        .saturating_sub(1)
        .saturating_mul(EXTRA_CALLS_PER_PART)
}

/// The extra calls a finished page's own evidence says it used.
fn parts_extra(parts: &PartsEvidence) -> u32 {
    u32::from(parts.planned.saturating_sub(1)).saturating_mul(EXTRA_CALLS_PER_PART)
}
```

Add the imports:

```rust
use clearcut_router::parts::EXTRA_CALLS_PER_PART;
use clearcut_router::{Carry, PageResult, PageRun, PageTask, PartsEvidence, Router};
```

- [ ] **Step 4: Store the split when a page ends or stops**

In `save_finished`, pass the evidence:

```rust
        let notes = page_notes(&result.outcome, result.text_dropped_bytes, result.parts.as_ref());
```

In the `PageRun::Interrupted` arm of `read_pages`, store the partial evidence with the carry:

```rust
                    let details = match &interruption.parts {
                        Some(parts) => PageDetails {
                            notes: Value::Array(parts_notes(parts)),
                            ..PageDetails::default()
                        },
                        None => PageDetails::default(),
                    };
                    if let Ok(carry) = serde_json::to_value(&interruption.carry) {
                        let saved = self
                            .store
                            .save_page_reading(
                                tenant,
                                document,
                                page,
                                &PageState::InProgress { carry },
                                &details,
                            )
                            .await;
                        if let Err(error) = saved {
                            tracing::warn!(%error, "the carry of an interrupted page could not be stored");
                        }
                    }
```

- [ ] **Step 5: Count the split in the document's allowance**

In `read_pages`, replace the call-rights line with:

```rust
        // A page read in parts is given extra calls. Only pages that have ended are counted
        // here: the router gives the page it is reading its own extra, once it has planned it.
        let extra: u32 = stored
            .iter()
            .filter(|page| !matches!(page.state, PageState::InProgress { .. }))
            .map(|page| part_extras(&page.details.notes))
            .sum();
        let mut calls_left = self.calls_left(tenant, document, pages, models, extra).await?;
```

In the `PageRun::Finished` arm, add the page's own extra once it has ended:

```rust
                    calls_left = calls_left.saturating_sub(counted_calls(&result));
                    // Now that the page has ended, the pages after it may use what its split
                    // was given. The router gave the page itself its extra while reading it.
                    if let Some(parts) = &result.parts {
                        calls_left = calls_left.saturating_add(parts_extra(parts));
                    }
```

Change `calls_left` to take and add the extra:

```rust
    /// How many calls the document may still make, including what its split pages were given.
    async fn calls_left(
        &self,
        tenant: TenantId,
        document: DocId,
        pages: u32,
        models: &Models<M>,
        extra: u32,
    ) -> Step<u32> {
        let used = self
            .store
            .document_spend(tenant, document)
            .await
            .map_err(|e| store_failure(&e))?
            .calls;
        Ok(models
            .settings
            .router
            .policy
            .document_call_cap(pages)
            .saturating_add(extra)
            .saturating_sub(used))
    }
```

- [ ] **Step 6: Tell the SDK what the new reasons are**

In `packages/sdk/src/types.ts`, at `WaitingPage.reason` (line 284), change the doc comment to:

```ts
  /** The fixed name of why it stopped, such as `tax_not_shown`, `part_failed` or `too_dense`. */
```

- [ ] **Step 7: Run the tests**

Run: `cargo test -p clearcut-pipeline`
Then: `cargo test -p clearcut-router`
Expected: PASS. The whole-page tests still see `parts: None` and the same call allowance.

- [ ] **Step 8: Commit**

```bash
git add crates/clearcut-pipeline packages/sdk/src/types.ts
git commit -m "feat(pipeline): record how a page was split and widen its call allowance"
```

---

### Task 9: Write it down, prove every gate, and deliver

**Files:**
- Modify: `docs/WORKLOG.md`
- Modify: `docs/adr/0018-model-router-and-escalation.md`
- Modify: `docs/PHASES.md`
- Modify: `docs/ARCHITECTURE.md`

**Interfaces:**
- Consumes: everything Tasks 1 to 8 built.
- Produces: the delivered slice on `main`, deployed and health-checked.

- [ ] **Step 1: Add the worklog entry**

At the top of `docs/WORKLOG.md`, under the intro line, insert:

```markdown
## 2026-11-05 (part 85): a dense page is read in parts
**Done**
- **A page that is too dense is split at its rows.** A page with at least 1 200 words and two rows is planned into 2 to 6 parts of about 500 words each. Words are ordered top to bottom and left to right, rows are assigned to parts as evenly as possible, and each seam is the midpoint between the last row of one part and the first of the next. The plan is a checked type: a stored plan that overlaps, leaves a gap, or does not cover the page's words and picture exactly is refused, so a damaged plan can only make a page be read again, never lose a line.
- **Every part is its own read.** A part is masked from its own words, cropped from its own band of the picture at the rung's resolution, labelled `part i of n of page p of P` in the prompt (`prompt-v4`, one added rule), and read by its own page state machine. Its attempt numbers continue the page's earlier calls, so a page's calls are still counted once.
- **The parts are merged in order.** The first non-empty fact wins, line readings are concatenated in page order, and a part that is still being read is not merged until it ends. Two parts that state different non-default values for the same fact send the page to a person as `part_conflict`. A part that cannot be read sends it as `part_failed`; a budget or a call cap keeps its own name. A page above 600 lines, a page whose reading cannot fit the 240 000-byte store limit, and a page with a part still cut off at the ceiling end `too_dense`.
- **A split page is given 3 extra calls per part beyond the first.** The router adds them for the page it is reading; the pipeline adds them to the document's allowance once the page ends, and a stored split page keeps its allowance across a restart. The per-page call cap is unchanged.
- **A page stopped between parts resumes at the first unfinished part.** Its plan and its earlier attempt number are kept, parts already read are not read again, and the page still stops honestly when the service fails.
- **The whole-page path is unchanged.** A page that is not dense is read and assessed exactly as before and its notes are the same.
- **One additive migration, no schema keeps its shape.** The reading schema stays `reading-v3`; the prompt moves to `prompt-v4`; every new stored fact is additive JSON in the page's notes and carry. Migration `0013` widens `page_readings_review_reason_check` so the three new reasons can be stored; it changes no data and no column.
- **Tests:** core planner (coverage, row grouping and stored-plan refusals), merge and its conflict rules, the worker's band crop and its refusal of an empty band, masking a part from its own words, the pack's part labels and prompt-v4 golden, the router's split, merge, conflict, budget and resume paths, and the pipeline's notes and allowance.
**Not done:** scans without OCR (B.2), the bounded repair round (C) and coverage evidence (D) are not started; a part that is itself cut off still follows the ladder and may end `too_long`; statement-shaped documents and page-spanning table headers remain out of scope.
**Deployed:** merged as the squash commit GitHub reports for the PR; the Johannesburg VM pulled `main`, rebuilt the server image and restarted it. `https://145-241-100-85.sslip.io/health` answers `{"status":"ok","version":"0.1.0"}` and `/` answers 200. No document was uploaded: the change is covered by the fakes, and an upload spends the owner's key.
```

- [ ] **Step 2: Amend ADR-0018's status line**

Append to the single Status paragraph, after the 4 November amendment sentence and before `· Date:`:

```text
· Amended 5 November 2026 (Slice B.1 of the long-invoice design): a page with at least 1 200 words and two rows is planned into 2 to 6 parts at row boundaries of about 500 words; each part is masked from its own words, cropped from its own band, sent as `part i of n` under `prompt-v4`, and read by its own machine with attempt numbers continuing the page's; the parts are merged in order, the first non-empty fact wins, and a disagreement between two parts or a part that cannot be read ends the page `needs_review` (`part_conflict`, `part_failed`); a page above 600 lines, a page whose reading cannot be stored, and a page with a part still cut off at the ceiling end `too_dense`; a split page is given 3 extra calls per part beyond the first.
```

- [ ] **Step 3: Update P2-07**

In `docs/PHASES.md`, in the P2-07 line, insert before `**Remain:**`:

```text
**Dense pages read in parts built** (Slice B.1 of the long-invoice design): a page with at least 1 200 words and two rows is split at row boundaries into 2 to 6 parts, each part masked and cropped from its own band, labelled `part i of n` under `prompt-v4`, read by its own machine with continuing attempt numbers, merged in order and given 3 extra calls per part beyond the first; `part_conflict`, `part_failed` and `too_dense` are fixed reasons for a person.
```

- [ ] **Step 4: Update ARCHITECTURE**

In the crate table of `docs/ARCHITECTURE.md`:
- The `clearcut-core` row's `escalation` clause gains: `, the plan that splits a dense page into row-boundary parts, and the merge that joins their readings in order and names a disagreement`.
- The `clearcut-router` row gains, after "and tells the page state machine": `; for a dense page it plans the parts, masks and crops each from its own band, reads each with its own machine, merges them in order, and reports the seams and a part that failed`.
- The `clearcut-pipeline` row gains, after "carrying what earlier pages established to the next": `, recording how each page was split and giving a split page its extra calls across restarts`.
- Replace each of those rows' test counts with the count the workspace run prints in Step 6.

- [ ] **Step 5: Prove the plan against the spec**

Re-read Sections 4 and 5 of `docs/superpowers/specs/2026-11-03-long-invoice-completeness-design.md` once and check each requirement against the code:
- 1 200 words and two rows trigger a split.
- 2 to 6 parts of about 500 words at row boundaries.
- Parts cover the page's text bytes and picture band exactly, with no overlap.
- Each part is masked from its own words and cropped at the rung's resolution.
- `prompt-v4` names the part; the schema is still `reading-v3`.
- The merge takes the first non-empty fact, concatenates lines, and reports `part_conflict`.
- `part_failed` and `too_dense` exist as fixed reasons and reach the SDK docs.
- The document's allowance grows by 3 per extra part, once per page, across restarts.
- A resumed page continues its attempt numbering and does not re-read finished parts.
- The whole-page path and every earlier test are unchanged.

If any requirement is missing, add the failing test and the code now, then return to Step 5.

- [ ] **Step 6: Run every gate**

Set the test database (see `CONTRIBUTING.md`):

```bash
docker start clearcut-pg 2>/dev/null || docker run -d --name clearcut-pg -e POSTGRES_USER=clearcut -e POSTGRES_HOST_AUTH_METHOD=trust -e POSTGRES_DB=clearcut -p 127.0.0.1:54329:5432 postgres:17
export DATABASE_URL=postgres://clearcut@127.0.0.1:54329/clearcut
```

Then:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm test
(cd services/worker && ruff check . && ruff format --check . && mypy && pytest -q)
```

Expected: every command exits 0. If a gate fails, fix the cause, commit the fix, and run all six again.

- [ ] **Step 7: Commit the documents**

```bash
git add docs/WORKLOG.md docs/adr/0018-model-router-and-escalation.md docs/PHASES.md docs/ARCHITECTURE.md
git commit -m "docs: record long-invoice Slice B.1"
```

- [ ] **Step 8: Open the PR, wait for CI, and squash-merge**

```bash
git push -u origin feat/long-invoices-b1-parts
gh pr create --title "Long invoices Slice B.1: read a dense page in parts" --body "$(cat <<'EOF'
## Summary
- Plans a page with at least 1 200 words and two rows into 2 to 6 parts at row boundaries.
- Masks and crops each part from its own band, sends it as `part i of n` (`prompt-v4`) and reads it with its own machine.
- Merges the parts in order; `part_conflict`, `part_failed` and `too_dense` send the page to a person.
- Gives a split page 3 extra calls per part beyond the first, once per page and across restarts.
- Records how a page was split in its notes; the reading schema stays `reading-v3`.
- Adds one migration that widens the stored review-reason check to the three new reasons; no data and no column change.

## Test plan
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `cargo test --workspace` (Postgres)
- [x] `npm test`
- [x] worker: ruff, format, mypy, pytest
EOF
)"
gh pr checks --watch
gh pr merge --squash --delete-branch
```

Expected: every check green, then the merge succeeds.

- [ ] **Step 9: Deploy and verify**

```bash
git checkout main
git pull --ff-only
ssh -i target/deploy/clearcut_oracle ubuntu@145.241.100.85 'cd ~/clearcut && git pull --ff-only && sudo docker compose --profile https up -d --build server'
curl -sS https://145-241-100-85.sslip.io/health
curl -sS -o /dev/null -w '%{http_code}\n' https://145-241-100-85.sslip.io/
```

Expected: health `{"status":"ok","version":"0.1.0"}` and `/` answers `200`. If the rebuild fails, read `sudo docker compose logs server --tail 100` on the VM and fix forward; do not leave the VM on a half-built image.

- [ ] **Step 10: Leave no stale refs**

```bash
git remote prune origin
git branch -a
```

Expected: no `feat/long-invoices-b1-parts` locally or on the remote. Record the deployed squash commit and the health answer in the worklog entry (replace "the squash commit GitHub reports for the PR" with the real short SHA and the restart time SAST), amend that line into a follow-up commit on `main`, and push it:

```bash
git add docs/WORKLOG.md
git commit -m "docs(worklog): record the Slice B.1 deploy and its health check"
git push
```

---
