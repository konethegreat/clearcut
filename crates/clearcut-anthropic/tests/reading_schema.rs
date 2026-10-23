// Written by Kone Tshivhinda & Claude
//! The schema the model is given for a page reading (ADR-0013) must be accepted
//! by the adapter's profile with room to spare, and must not change by accident:
//! a changed schema breaks the prompt cache and makes gold-set scores
//! incomparable. To change it on purpose, bump `READING_SHAPE_VERSION`, then
//! run `UPDATE_GOLDEN=1 cargo test -p clearcut-anthropic --test reading_schema`.

use clearcut_anthropic::schema::profile;
use clearcut_core::reading::{reading_schema, READING_SHAPE_VERSION};
use serde_json::Value;

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/reading-v1.schema.json");

fn schema() -> Value {
    serde_json::to_value(reading_schema()).unwrap()
}

#[test]
fn the_reading_schema_is_accepted_with_no_optional_and_no_union_properties() {
    let profiled = profile(&schema()).expect("the API must accept the reading schema");
    assert_eq!(profiled.optional_properties, 0, "every field is required");
    assert_eq!(profiled.union_properties, 0, "no unions");
}

#[test]
fn nothing_has_to_be_removed_from_the_reading_schema() {
    let profiled = profile(&schema()).unwrap();
    assert!(profiled.removed.is_empty(), "{:?}", profiled.removed);
}

#[test]
fn the_profiled_schema_closes_every_object() {
    let profiled = profile(&schema()).unwrap().schema;
    fn closed(value: &Value) {
        match value {
            Value::Object(map) => {
                if map.get("type") == Some(&Value::String("object".into())) || map.contains_key("properties")
                {
                    assert_eq!(
                        map.get("additionalProperties"),
                        Some(&Value::Bool(false)),
                        "{map:?}"
                    );
                }
                map.values().for_each(closed);
            }
            Value::Array(items) => items.iter().for_each(closed),
            _ => {}
        }
    }
    closed(&profiled);
}

#[test]
fn every_property_of_every_object_is_required() {
    fn check(value: &Value) {
        if let Value::Object(map) = value {
            if let Some(Value::Object(properties)) = map.get("properties") {
                let required: Vec<&str> = map["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                for name in properties.keys() {
                    assert!(required.contains(&name.as_str()), "{name} is optional");
                }
            }
            map.values().for_each(check);
        } else if let Value::Array(items) = value {
            items.iter().for_each(check);
        }
    }
    let schema = schema();
    check(&schema);
}

#[test]
fn the_schema_asks_for_nothing_the_model_must_not_decide() {
    let text = schema().to_string();
    for forbidden in [
        "confidence",
        "locator",
        "\"page\"",
        "extractor",
        "\"document\"",
        "minor",
    ] {
        assert!(!text.contains(forbidden), "the schema mentions {forbidden}");
    }
}

#[test]
fn the_version_names_the_shape() {
    assert_eq!(READING_SHAPE_VERSION, "reading-v1");
    assert!(GOLDEN.ends_with("reading-v1.schema.json"));
}

#[test]
fn the_schema_has_not_changed_by_accident() {
    let current = serde_json::to_string_pretty(&schema()).unwrap() + "\n";
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(GOLDEN, &current).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(GOLDEN)
        .expect("tests/reading-v1.schema.json is missing; run with UPDATE_GOLDEN=1")
        .replace("\r\n", "\n");
    assert_eq!(
        current, golden,
        "the reading schema changed; if that is intended, bump READING_SHAPE_VERSION and rerun with UPDATE_GOLDEN=1"
    );
}
