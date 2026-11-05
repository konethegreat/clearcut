// Written by Kone Tshivhinda & Claude
//! The planner: when a page is split, how many parts, and whether every word lands in one.

use super::parts::{plan_parts, PartsPlan, PositionedWord, MAX_PARTS, MIN_WORDS};
use serde_json::{json, Value};

/// `count` words of the same width, `per_row` to a row, in reading order, spread down the page.
fn layout(count: usize, per_row: usize) -> (String, Vec<PositionedWord>) {
    let rows = count.div_ceil(per_row);
    // At most half the row spacing, so every word stays on the page (its bottom stays at or
    // under 1 000) and neighbouring rows never merge into one.
    let height = u16::try_from((490 / rows).max(1)).unwrap();
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
        let top = u16::try_from(10 + (row * 980) / rows).unwrap();
        words.push(PositionedWord {
            start,
            end,
            x: u16::try_from((column % 25) * 38).unwrap(),
            top,
            bottom: top.checked_add(height).unwrap(),
        });
    }
    (text, words)
}

/// Every part's picture range sits on the page, in order, and the parts cover it end to end.
fn assert_page_geometry(plan: &PartsPlan) {
    let parts = plan.parts();
    assert_eq!(parts[0].top(), 0);
    assert_eq!(parts[parts.len() - 1].bottom(), 1_000);
    for part in parts {
        assert!(part.top() < part.bottom(), "part {} is upside down", part.index());
        assert!(part.bottom() <= 1_000, "part {} runs off the page", part.index());
    }
    for pair in parts.windows(2) {
        assert_eq!(pair[0].bottom(), pair[1].top(), "the parts are not contiguous");
    }
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
    assert_eq!(
        shape,
        vec![(1, 3, 0, 9, 400), (2, 3, 10, 19, 400), (3, 3, 20, 29, 400)]
    );
    assert_page_geometry(&plan);
    // Row 9 ends at 320, row 10 begins at 336: the seam is the midpoint.
    assert_eq!(plan.parts()[0].bottom(), 328);
    assert_eq!(plan.parts()[1].top(), 328);
    assert_eq!(plan.parts()[1].bottom(), 654);
    assert_eq!(plan.parts()[2].top(), 654);
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
    // Every part must be a real slice of the page, not geometry that runs past 1 000.
    assert_page_geometry(&plan);
}

#[test]
fn every_word_lands_in_one_part_and_the_ranges_are_ordered_and_do_not_overlap() {
    for (count, per_row) in [(1_200, 40), (2_400, 60), (7_000, 50)] {
        let (text, words) = layout(count, per_row);
        let plan = plan_parts(&words, text.len()).unwrap();
        assert_page_geometry(&plan);
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
    // Six hundred rows of two. The text is column-major: the left column's words are the first
    // six hundred slots of the text and the right column's the next six hundred, while the
    // boxes are row-major. Planning by row therefore takes the right column's tail of a part
    // before the left column's head of the next part, so the first part's byte range runs past
    // the second part's start. That is an invalid layout: the plan's own checks refuse it and
    // the page is read whole.
    let rows = 600usize;
    let mut text = String::new();
    let mut starts = Vec::with_capacity(rows * 2);
    for slot in 0..rows * 2 {
        if slot > 0 {
            text.push(' ');
        }
        starts.push(text.len());
        text.push_str(&format!("w{slot:05}"));
    }
    let mut words = Vec::with_capacity(rows * 2);
    for row in 0..rows {
        let top = u16::try_from(10 + (row * 980) / rows).unwrap();
        for (column, slot) in [row, rows + row].into_iter().enumerate() {
            words.push(PositionedWord {
                start: starts[slot],
                end: starts[slot] + 6,
                x: if column == 0 { 100 } else { 600 },
                top,
                bottom: top + 1,
            });
        }
    }
    assert_eq!(words.len(), MIN_WORDS);
    assert!(plan_parts(&words, text.len()).is_none());

    // The same boxes with the text in box order do plan, so the refusal above comes from the
    // text order and not from a page that is too small to split.
    let mut in_order = words.clone();
    for (slot, word) in in_order.iter_mut().enumerate() {
        word.start = starts[slot];
        word.end = starts[slot] + 6;
    }
    assert!(plan_parts(&in_order, text.len()).is_some());
}

#[test]
fn a_word_that_cannot_be_sliced_plans_nothing() {
    let (text, words) = layout(1_200, 40);
    let check = |change: &dyn Fn(&mut PositionedWord)| {
        let mut broken = words.clone();
        // The last word, so no later row can drag the damaged box back inside a row.
        change(&mut broken[1_199]);
        assert!(plan_parts(&broken, text.len()).is_none());
    };
    // A word with no width.
    check(&|word| word.end = word.start);
    // A word that ends past the page's text.
    check(&|word| word.end = text.len() + 1);
    // A word with no height.
    check(&|word| word.bottom = word.top);
    // A word that hangs below the page.
    check(&|word| word.bottom = 1_001);
}

#[test]
fn a_part_is_found_by_its_number() {
    let (text, words) = layout(1_200, 40);
    let plan = plan_parts(&words, text.len()).unwrap();
    assert_eq!(plan.part(2).map(|part| part.index()), Some(2));
    assert!(plan.part(4).is_none());
    assert!(plan.part(0).is_none());
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

#[test]
fn a_stored_plan_with_overflowing_numbers_is_refused_not_panicked() {
    let (text, words) = layout(1_200, 40);
    let plan = plan_parts(&words, text.len()).unwrap();
    let value = serde_json::to_value(&plan).unwrap();

    let mut huge_row = value.clone();
    huge_row["parts"][0]["last_row"] = json!(usize::MAX);
    assert!(serde_json::from_value::<super::parts::PartsPlan>(huge_row).is_err());

    // One sum that fits in usize, then a second that does not.
    let mut huge_words = value.clone();
    huge_words["parts"][0]["words"] = json!(usize::MAX);
    huge_words["parts"][1]["words"] = json!(usize::MAX);
    assert!(serde_json::from_value::<super::parts::PartsPlan>(huge_words).is_err());
}
