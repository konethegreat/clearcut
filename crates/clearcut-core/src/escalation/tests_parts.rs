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
    assert_eq!(
        shape,
        vec![(1, 3, 0, 9, 400), (2, 3, 10, 19, 400), (3, 3, 20, 29, 400)]
    );
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
