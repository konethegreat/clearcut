// Written by Kone Tshivhinda & Claude
use super::*;
use serde_json::{json, Value};

fn profiled(schema: Value) -> Profile {
    profile(&schema).expect("the schema should be accepted")
}

fn refused(schema: Value) -> SchemaError {
    profile(&schema).expect_err("the schema should be refused")
}

fn simple() -> Value {
    json!({
        "type": "object",
        "properties": {
            "number": { "type": "string" },
            "total": { "type": "string" }
        },
        "required": ["number", "total"]
    })
}

// ---------- closed objects ----------

#[test]
fn every_object_is_closed() {
    let result = profiled(json!({
        "type": "object",
        "properties": {
            "supplier": {
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            },
            "lines": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"]
                }
            }
        },
        "required": ["supplier", "lines"]
    }));
    let s = &result.schema;
    assert_eq!(s["additionalProperties"], json!(false));
    assert_eq!(s["properties"]["supplier"]["additionalProperties"], json!(false));
    assert_eq!(
        s["properties"]["lines"]["items"]["additionalProperties"],
        json!(false)
    );
}

#[test]
fn an_object_that_was_left_open_is_closed_anyway() {
    let mut schema = simple();
    schema["additionalProperties"] = json!(true);
    assert_eq!(profiled(schema).schema["additionalProperties"], json!(false));
}

#[test]
fn objects_inside_definitions_and_unions_are_closed_too() {
    let result = profiled(json!({
        "type": "object",
        "properties": {
            "party": { "$ref": "#/$defs/party" },
            "either": { "anyOf": [
                { "type": "object", "properties": { "a": { "type": "string" } }, "required": ["a"] },
                { "type": "null" }
            ] }
        },
        "required": ["party", "either"],
        "$defs": {
            "party": {
                "type": "object",
                "properties": { "name": { "type": "string" } },
                "required": ["name"]
            }
        }
    }));
    let s = &result.schema;
    assert_eq!(s["$defs"]["party"]["additionalProperties"], json!(false));
    assert_eq!(
        s["properties"]["either"]["anyOf"][0]["additionalProperties"],
        json!(false)
    );
    assert_eq!(s["properties"]["party"]["$ref"], json!("#/$defs/party"));
}

#[test]
fn the_root_must_be_an_object() {
    for schema in [
        json!({ "type": "string" }),
        json!({ "type": "array", "items": { "type": "string" } }),
        json!({ "anyOf": [{ "type": "object" }] }),
        json!(true),
        json!("object"),
    ] {
        assert!(
            matches!(refused(schema), SchemaError::RootNotAnObject),
            "root must be an object"
        );
    }
}

// ---------- unsupported constraints are removed and said so ----------

#[test]
fn every_unsupported_keyword_is_removed() {
    let keywords = [
        ("minimum", json!(1)),
        ("maximum", json!(10)),
        ("exclusiveMinimum", json!(0)),
        ("exclusiveMaximum", json!(11)),
        ("multipleOf", json!(5)),
        ("minLength", json!(1)),
        ("maxLength", json!(20)),
        ("maxItems", json!(3)),
        ("uniqueItems", json!(true)),
        ("pattern", json!("^[A-Z]+$")),
    ];
    for (keyword, value) in keywords {
        let mut field = json!({ "type": "string" });
        field[keyword] = value.clone();
        let result = profiled(json!({
            "type": "object",
            "properties": { "x": field },
            "required": ["x"]
        }));
        let x = &result.schema["properties"]["x"];
        assert!(x.get(keyword).is_none(), "{keyword} should be removed");
        assert_eq!(x["type"], json!("string"), "{keyword}: the rest stays");
        let note = x["description"].as_str().unwrap_or_default();
        assert!(
            note.contains("not enforced here") && note.contains(keyword),
            "{keyword}: the model is told, got {note:?}"
        );
        assert_eq!(result.removed.len(), 1, "{keyword}");
        assert_eq!(result.removed[0].keyword, keyword);
        assert_eq!(result.removed[0].path, "/properties/x");
    }
}

#[test]
fn the_note_keeps_the_original_description_and_lists_every_removed_rule_in_a_fixed_order() {
    let result = profiled(json!({
        "type": "object",
        "properties": {
            "qty": {
                "type": "integer",
                "description": "How many.",
                "maximum": 99,
                "minimum": 1
            }
        },
        "required": ["qty"]
    }));
    assert_eq!(
        result.schema["properties"]["qty"]["description"],
        json!("How many. (not enforced here: minimum 1, maximum 99)")
    );
}

#[test]
fn a_field_without_a_description_gets_one() {
    let result = profiled(json!({
        "type": "object",
        "properties": { "code": { "type": "string", "pattern": "^A" } },
        "required": ["code"]
    }));
    assert_eq!(
        result.schema["properties"]["code"]["description"],
        json!("(not enforced here: pattern \"^A\")")
    );
}

#[test]
fn min_items_of_zero_or_one_is_kept_and_larger_is_removed() {
    for (value, kept) in [(0, true), (1, true), (2, false), (50, false)] {
        let result = profiled(json!({
            "type": "object",
            "properties": {
                "list": { "type": "array", "items": { "type": "string" }, "minItems": value }
            },
            "required": ["list"]
        }));
        let list = &result.schema["properties"]["list"];
        assert_eq!(list.get("minItems").is_some(), kept, "minItems {value}");
        assert_eq!(result.removed.len(), usize::from(!kept), "minItems {value}");
    }
}

#[test]
fn only_the_formats_the_api_knows_are_kept() {
    for format in [
        "date",
        "date-time",
        "time",
        "duration",
        "email",
        "hostname",
        "uri",
        "ipv4",
        "ipv6",
        "uuid",
    ] {
        let result = profiled(json!({
            "type": "object",
            "properties": { "x": { "type": "string", "format": format } },
            "required": ["x"]
        }));
        assert_eq!(result.schema["properties"]["x"]["format"], json!(format));
        assert!(result.removed.is_empty(), "{format}");
    }
    let result = profiled(json!({
        "type": "object",
        "properties": { "x": { "type": "string", "format": "iri-reference" } },
        "required": ["x"]
    }));
    assert!(result.schema["properties"]["x"].get("format").is_none());
    assert_eq!(result.removed[0].keyword, "format");
}

#[test]
fn the_schema_marker_is_dropped_without_a_note() {
    let mut schema = simple();
    schema["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    let result = profiled(schema);
    assert!(result.schema.get("$schema").is_none());
    assert!(result.schema.get("description").is_none());
    assert!(result.removed.is_empty());
}

#[test]
fn a_property_named_like_a_keyword_is_a_property_not_a_rule() {
    let result = profiled(json!({
        "type": "object",
        "properties": {
            "minimum": { "type": "string" },
            "pattern": { "type": "string" },
            "maxItems": { "type": "string" },
            "$schema": { "type": "string" }
        },
        "required": ["minimum", "pattern", "maxItems", "$schema"]
    }));
    for name in ["minimum", "pattern", "maxItems", "$schema"] {
        assert!(
            result.schema["properties"].get(name).is_some(),
            "{name} must survive"
        );
    }
    assert!(result.removed.is_empty());
}

#[test]
fn constraints_inside_items_unions_and_definitions_are_found() {
    let result = profiled(json!({
        "type": "object",
        "properties": {
            "tags": { "type": "array", "items": { "type": "string", "maxLength": 5 } },
            "maybe": { "anyOf": [{ "type": "integer", "minimum": 0 }, { "type": "null" }] },
            "money": { "$ref": "#/$defs/money" }
        },
        "required": ["tags", "maybe", "money"],
        "$defs": { "money": { "type": "string", "pattern": "^[0-9]+$" } }
    }));
    let mut paths: Vec<&str> = result.removed.iter().map(|r| r.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(
        paths,
        vec![
            "/$defs/money",
            "/properties/maybe/anyOf/0",
            "/properties/tags/items"
        ]
    );
}

// ---------- refusals ----------

#[test]
fn a_recursive_schema_is_refused() {
    let direct = refused(json!({
        "type": "object",
        "properties": { "child": { "$ref": "#/$defs/node" } },
        "required": ["child"],
        "$defs": {
            "node": {
                "type": "object",
                "properties": { "next": { "$ref": "#/$defs/node" } },
                "required": ["next"]
            }
        }
    }));
    assert!(matches!(direct, SchemaError::Recursive { .. }), "{direct}");

    let through_two = refused(json!({
        "type": "object",
        "properties": { "a": { "$ref": "#/$defs/a" } },
        "required": ["a"],
        "$defs": {
            "a": { "type": "object", "properties": { "b": { "$ref": "#/$defs/b" } }, "required": ["b"] },
            "b": { "type": "object", "properties": { "a": { "$ref": "#/$defs/a" } }, "required": ["a"] }
        }
    }));
    assert!(matches!(through_two, SchemaError::Recursive { .. }));

    let root = refused(json!({
        "type": "object",
        "properties": { "me": { "$ref": "#" } },
        "required": ["me"]
    }));
    assert!(matches!(
        root,
        SchemaError::Recursive { .. } | SchemaError::ExternalRef { .. }
    ));
}

#[test]
fn the_same_definition_used_twice_is_not_recursion() {
    let result = profiled(json!({
        "type": "object",
        "properties": {
            "from": { "$ref": "#/$defs/party" },
            "to": { "$ref": "#/$defs/party" }
        },
        "required": ["from", "to"],
        "$defs": {
            "party": { "type": "object", "properties": { "name": { "type": "string" } }, "required": ["name"] }
        }
    }));
    assert_eq!(
        result.schema["properties"]["from"]["$ref"],
        json!("#/$defs/party")
    );
}

#[test]
fn an_external_or_unknown_reference_is_refused() {
    for target in [
        "https://example.com/schema.json",
        "other.json#/x",
        "#/$defs/missing",
        "#/properties/number",
    ] {
        let error = refused(json!({
            "type": "object",
            "properties": { "x": { "$ref": target } },
            "required": ["x"],
            "$defs": {}
        }));
        assert!(
            matches!(
                error,
                SchemaError::ExternalRef { .. } | SchemaError::UnknownRef { .. }
            ),
            "{target}: {error}"
        );
    }
}

#[test]
fn all_of_beside_a_reference_is_refused() {
    let error = refused(json!({
        "type": "object",
        "properties": { "x": { "allOf": [{ "$ref": "#/$defs/a" }, { "type": "object" }] } },
        "required": ["x"],
        "$defs": { "a": { "type": "object", "properties": {}, "required": [] } }
    }));
    assert!(matches!(error, SchemaError::AllOfWithRef { .. }), "{error}");

    // allOf without any $ref is fine.
    let ok = profiled(json!({
        "type": "object",
        "properties": { "x": { "allOf": [{ "type": "string" }] } },
        "required": ["x"]
    }));
    assert!(ok.schema["properties"]["x"]["allOf"].is_array());
}

#[test]
fn an_enum_of_objects_or_lists_is_refused() {
    for bad in [json!([{ "a": 1 }]), json!([[1, 2]]), json!(["ok", { "a": 1 }])] {
        let error = refused(json!({
            "type": "object",
            "properties": { "x": { "enum": bad } },
            "required": ["x"]
        }));
        assert!(matches!(error, SchemaError::NonPrimitiveEnum { .. }), "{error}");
    }
    let ok = profiled(json!({
        "type": "object",
        "properties": { "x": { "enum": ["a", 1, 2.5, true, null] } },
        "required": ["x"]
    }));
    assert_eq!(ok.schema["properties"]["x"]["enum"].as_array().unwrap().len(), 5);
}

#[test]
fn a_refusal_says_where() {
    let error = refused(json!({
        "type": "object",
        "properties": { "inner": { "type": "object", "properties": {
            "x": { "enum": [{ "a": 1 }] } }, "required": ["x"] } },
        "required": ["inner"]
    }));
    assert!(
        error.to_string().contains("/properties/inner/properties/x"),
        "{error}"
    );
}

#[test]
fn a_schema_too_deep_or_too_large_is_refused_instead_of_looping() {
    let mut deep = json!({ "type": "string" });
    for _ in 0..80 {
        deep = json!({ "type": "object", "properties": { "n": deep }, "required": ["n"] });
    }
    assert!(matches!(refused(deep), SchemaError::TooDeep { .. }));

    // A definition that mentions the previous one twice, sixteen times over: over a hundred thousand uses.
    let mut defs = serde_json::Map::new();
    defs.insert(
        "d0".into(),
        json!({ "type": "object", "properties": { "v": { "type": "string" } }, "required": ["v"] }),
    );
    for i in 1..=16 {
        defs.insert(
            format!("d{i}"),
            json!({
                "type": "object",
                "properties": {
                    "a": { "$ref": format!("#/$defs/d{}", i - 1) },
                    "b": { "$ref": format!("#/$defs/d{}", i - 1) }
                },
                "required": ["a", "b"]
            }),
        );
    }
    let error = refused(json!({
        "type": "object",
        "properties": { "top": { "$ref": "#/$defs/d16" } },
        "required": ["top"],
        "$defs": defs
    }));
    assert!(matches!(error, SchemaError::TooLarge { .. }), "{error}");
}

// ---------- the counts ----------

fn with_optionals(n: usize) -> Value {
    let mut props = serde_json::Map::new();
    for i in 0..n {
        props.insert(format!("f{i}"), json!({ "type": "string" }));
    }
    json!({ "type": "object", "properties": props, "required": [] })
}

#[test]
fn optional_properties_are_counted_and_the_limit_is_twenty_four() {
    let ok = profiled(with_optionals(24));
    assert_eq!(ok.optional_properties, 24);
    match refused(with_optionals(25)) {
        SchemaError::TooManyOptional { found, limit } => {
            assert_eq!((found, limit), (25, 24));
        }
        other => panic!("wrong error: {other}"),
    }
}

#[test]
fn required_properties_are_not_optional() {
    let mut schema = with_optionals(30);
    let names: Vec<String> = (0..30).map(|i| format!("f{i}")).collect();
    schema["required"] = json!(names);
    let result = profiled(schema);
    assert_eq!(result.optional_properties, 0);
}

#[test]
fn a_shared_definition_counts_every_time_it_is_used() {
    let schema = |uses: usize| {
        let mut props = serde_json::Map::new();
        let mut required = Vec::new();
        for i in 0..uses {
            props.insert(format!("p{i}"), json!({ "$ref": "#/$defs/opt" }));
            required.push(format!("p{i}"));
        }
        json!({
            "type": "object",
            "properties": props,
            "required": required,
            "$defs": {
                "opt": { "type": "object", "properties": { "a": { "type": "string" } }, "required": [] }
            }
        })
    };
    assert_eq!(profiled(schema(24)).optional_properties, 24);
    assert!(matches!(
        refused(schema(25)),
        SchemaError::TooManyOptional { found: 25, .. }
    ));
}

fn with_unions(n: usize) -> Value {
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();
    for i in 0..n {
        props.insert(
            format!("u{i}"),
            if i % 2 == 0 {
                json!({ "anyOf": [{ "type": "string" }, { "type": "null" }] })
            } else {
                json!({ "type": ["string", "null"] })
            },
        );
        required.push(format!("u{i}"));
    }
    json!({ "type": "object", "properties": props, "required": required })
}

#[test]
fn union_types_are_counted_and_the_limit_is_sixteen() {
    let ok = profiled(with_unions(16));
    assert_eq!(ok.union_properties, 16);
    match refused(with_unions(17)) {
        SchemaError::TooManyUnions { found, limit } => assert_eq!((found, limit), (17, 16)),
        other => panic!("wrong error: {other}"),
    }
}

#[test]
fn a_plain_type_list_of_one_is_not_a_union() {
    let result = profiled(json!({
        "type": "object",
        "properties": { "x": { "type": ["string"] } },
        "required": ["x"]
    }));
    assert_eq!(result.union_properties, 0);
}

// ---------- determinism ----------

#[test]
fn the_same_schema_gives_byte_identical_output() {
    let schema = json!({
        "type": "object",
        "properties": {
            "b": { "type": "string", "maxLength": 3, "description": "B." },
            "a": { "type": "integer", "minimum": 0, "maximum": 9 }
        },
        "required": ["a", "b"]
    });
    let first = serde_json::to_string(&profiled(schema.clone()).schema).unwrap();
    for _ in 0..5 {
        assert_eq!(
            serde_json::to_string(&profiled(schema.clone()).schema).unwrap(),
            first
        );
    }
}

#[test]
fn profiling_a_profiled_schema_changes_nothing() {
    let once = profiled(json!({
        "type": "object",
        "properties": {
            "a": { "type": "integer", "minimum": 0, "description": "A." },
            "o": { "type": "object", "properties": {}, "required": [] }
        },
        "required": ["a", "o"]
    }));
    let twice = profiled(once.schema.clone());
    assert_eq!(once.schema, twice.schema);
    assert!(twice.removed.is_empty());
}

#[test]
fn the_callers_schema_is_not_modified() {
    let schema = json!({
        "type": "object",
        "properties": { "a": { "type": "integer", "minimum": 0 } },
        "required": ["a"]
    });
    let before = schema.clone();
    let _ = profile(&schema).unwrap();
    assert_eq!(schema, before);
}

#[test]
fn a_reference_must_name_exactly_one_definition() {
    for target in ["#/$defs/a/b", "#/$defs/", "#/$defs", "#/definitions/"] {
        let error = refused(json!({
            "type": "object",
            "properties": { "x": { "$ref": target } },
            "required": ["x"],
            "$defs": { "a": { "type": "string" } }
        }));
        assert!(
            matches!(error, SchemaError::ExternalRef { .. }),
            "{target}: {error}"
        );
    }
}

#[test]
fn one_of_unions_are_counted_like_any_of() {
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();
    for i in 0..17 {
        props.insert(
            format!("u{i}"),
            json!({ "oneOf": [{ "type": "string" }, { "type": "integer" }] }),
        );
        required.push(format!("u{i}"));
    }
    let error = refused(json!({ "type": "object", "properties": props, "required": required }));
    assert!(
        matches!(error, SchemaError::TooManyUnions { found: 17, .. }),
        "{error}"
    );
}
