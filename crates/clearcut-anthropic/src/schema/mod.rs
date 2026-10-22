// Written by Kone Tshivhinda & Claude
//! The schema profile (ADR-0012 §4).
//!
//! Anthropic's structured outputs accept only a **subset** of JSON Schema
//! (research 06 §6). We do not hand the caller's schema over and hope: [`profile`]
//! turns it into one the API accepts, or refuses it, here, in our own code, with
//! a precise reason. That is cheaper and clearer than a 400 from the API.
//!
//! What it does:
//! - closes every object (`additionalProperties: false`);
//! - removes the constraints the API does not support, and **says so** in the
//!   field's `description` ("not enforced here: minimum 1"), because
//!   `clearcut-core` enforces them afterwards anyway and the model should still be told;
//! - refuses what cannot be expressed: recursion, references that leave the
//!   schema, `allOf` beside a `$ref`, enums of objects or lists, and schemas that
//!   are too deep, too large, or over the API's limits for optional (24) and
//!   union (16) properties.
//!
//! It is deterministic (the same input always gives the same bytes, so the
//! prompt cache and the API's compiled grammar are reused), it never changes the
//! caller's schema, and profiling an already profiled schema changes nothing.

use serde_json::{Map, Value};

/// The API allows at most this many optional properties in one schema (research 06 §6).
pub const MAX_OPTIONAL_PROPERTIES: usize = 24;
/// The API allows at most this many union-typed properties in one schema.
pub const MAX_UNION_PROPERTIES: usize = 16;
/// Schemas nested deeper than this are refused (a safety net against runaway input).
const MAX_DEPTH: usize = 64;
/// Counting follows every use of a shared definition; past this many nodes the
/// schema is refused instead of being walked for ever.
const MAX_NODES: usize = 20_000;

/// Keywords the API does not support, in the fixed order their notes are written.
const UNSUPPORTED: [&str; 12] = [
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "uniqueItems",
    "pattern",
    "format",
];

/// String formats the API understands (research 06 §6).
const SUPPORTED_FORMATS: [&str; 10] = [
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
];

/// A rule that was taken out of the schema because the API cannot enforce it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// Where in the schema (a JSON pointer such as `/properties/total`).
    pub path: String,
    /// The keyword that was removed, such as `minimum`.
    pub keyword: &'static str,
}

/// A schema the API accepts, plus what was done to get it there.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// The schema to send as `output_config.format.schema`.
    pub schema: Value,
    /// Every rule that was removed, so the caller can check it is enforced elsewhere.
    pub removed: Vec<Removal>,
    /// Optional properties counted against the API's limit (every use of a shared definition counts).
    pub optional_properties: usize,
    /// Union-typed properties counted against the API's limit.
    pub union_properties: usize,
}

/// Why a schema cannot be sent to the API. Every message names where.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaError {
    #[error("the schema must describe an object (`type: object`) at the top")]
    RootNotAnObject,
    #[error("the schema is recursive through the definition `{definition}`, which the API cannot express")]
    Recursive { definition: String },
    #[error(
        "at `{path}`: the reference `{reference}` leaves the schema or is not a `#/$defs/name` reference"
    )]
    ExternalRef { path: String, reference: String },
    #[error("the reference `{reference}` points at a definition that does not exist")]
    UnknownRef { reference: String },
    #[error("at `{path}`: `allOf` beside a `$ref` is not supported by the API")]
    AllOfWithRef { path: String },
    #[error("at `{path}`: an `enum` may hold only strings, numbers, booleans and null")]
    NonPrimitiveEnum { path: String },
    #[error("at `{path}`: the schema is nested too deeply")]
    TooDeep { path: String },
    #[error("the schema is too large once shared definitions are counted at every use (over {limit} parts)")]
    TooLarge { limit: usize },
    #[error("the schema has {found} optional properties; the API allows {limit}")]
    TooManyOptional { found: usize, limit: usize },
    #[error("the schema has {found} union-typed properties; the API allows {limit}")]
    TooManyUnions { found: usize, limit: usize },
}

/// Turns `schema` into one the API accepts, or says why it cannot.
pub fn profile(schema: &Value) -> Result<Profile, SchemaError> {
    let is_object_root = schema
        .as_object()
        .and_then(|map| map.get("type"))
        .is_some_and(|t| t == "object");
    if !is_object_root {
        return Err(SchemaError::RootNotAnObject);
    }

    let mut rewriter = Rewriter { removed: Vec::new() };
    let rewritten = rewriter.node(schema, "", 0, true)?;

    let mut counter = Counter {
        root: &rewritten,
        nodes: 0,
        optional: 0,
        unions: 0,
    };
    counter.node(&rewritten, 0, &mut Vec::new())?;
    let (optional, unions) = (counter.optional, counter.unions);
    if optional > MAX_OPTIONAL_PROPERTIES {
        return Err(SchemaError::TooManyOptional {
            found: optional,
            limit: MAX_OPTIONAL_PROPERTIES,
        });
    }
    if unions > MAX_UNION_PROPERTIES {
        return Err(SchemaError::TooManyUnions {
            found: unions,
            limit: MAX_UNION_PROPERTIES,
        });
    }

    Ok(Profile {
        schema: rewritten,
        removed: rewriter.removed,
        optional_properties: optional,
        union_properties: unions,
    })
}

/// Escapes a property name for use inside a JSON pointer (`~` and `/`).
fn escape(name: &str) -> String {
    name.replace('~', "~0").replace('/', "~1")
}

/// Which definition a `$ref` points at: `Some(("$defs", "name"))`, or `None` if
/// it is not a local definition reference.
fn local_definition(reference: &str) -> Option<(&'static str, &str)> {
    for folder in ["$defs", "definitions"] {
        if let Some(name) = reference.strip_prefix(&format!("#/{folder}/")) {
            if !name.is_empty() && !name.contains('/') {
                return Some((folder, name));
            }
        }
    }
    None
}

/// Pass one: rewrite each schema node (no references followed).
struct Rewriter {
    removed: Vec<Removal>,
}

impl Rewriter {
    fn node(
        &mut self,
        schema: &Value,
        path: &str,
        depth: usize,
        is_root: bool,
    ) -> Result<Value, SchemaError> {
        // `true` / `false` schemas and anything else that is not an object pass through.
        let Some(map) = schema.as_object() else {
            return Ok(schema.clone());
        };
        if depth > MAX_DEPTH {
            return Err(SchemaError::TooDeep {
                path: path.to_string(),
            });
        }
        let mut out = map.clone();
        if is_root {
            out.remove("$schema");
        }

        if let Some(reference) = out.get("$ref") {
            let text = reference.as_str().unwrap_or_default();
            if local_definition(text).is_none() {
                return Err(SchemaError::ExternalRef {
                    path: path.to_string(),
                    reference: text.to_string(),
                });
            }
        }
        if let Some(Value::Array(parts)) = out.get("allOf") {
            if out.contains_key("$ref") || parts.iter().any(|p| p.get("$ref").is_some()) {
                return Err(SchemaError::AllOfWithRef {
                    path: path.to_string(),
                });
            }
        }
        if let Some(Value::Array(values)) = out.get("enum") {
            if values.iter().any(|v| v.is_array() || v.is_object()) {
                return Err(SchemaError::NonPrimitiveEnum {
                    path: path.to_string(),
                });
            }
        }

        // Take out what the API cannot enforce, and remember what was said.
        let mut notes: Vec<String> = Vec::new();
        for keyword in UNSUPPORTED {
            let Some(value) = out.get(keyword).cloned() else {
                continue;
            };
            let keep = match keyword {
                "minItems" => value.as_u64().is_some_and(|n| n <= 1),
                "format" => value.as_str().is_some_and(|f| SUPPORTED_FORMATS.contains(&f)),
                _ => false,
            };
            if !keep {
                out.remove(keyword);
                notes.push(format!("{keyword} {value}"));
                self.removed.push(Removal {
                    path: path.to_string(),
                    keyword,
                });
            }
        }
        if !notes.is_empty() {
            let note = format!("(not enforced here: {})", notes.join(", "));
            let description = match out.get("description").and_then(Value::as_str) {
                Some(existing) if !existing.is_empty() => format!("{existing} {note}"),
                _ => note,
            };
            out.insert("description".into(), Value::String(description));
        }

        // Close every object.
        let says_object = match out.get("type") {
            Some(Value::String(t)) => t == "object",
            Some(Value::Array(types)) => types.iter().any(|t| t == "object"),
            _ => false,
        };
        if says_object || out.contains_key("properties") {
            out.insert("additionalProperties".into(), Value::Bool(false));
        }

        // Then the schemas inside this one.
        for folder in ["properties", "$defs", "definitions"] {
            if let Some(Value::Object(children)) = out.get(folder).cloned() {
                let mut rewritten = Map::new();
                for (name, child) in &children {
                    let child_path = format!("{path}/{folder}/{}", escape(name));
                    rewritten.insert(name.clone(), self.node(child, &child_path, depth + 1, false)?);
                }
                out.insert(folder.into(), Value::Object(rewritten));
            }
        }
        for keyword in ["anyOf", "oneOf", "allOf", "prefixItems"] {
            if let Some(Value::Array(children)) = out.get(keyword).cloned() {
                let mut rewritten = Vec::with_capacity(children.len());
                for (index, child) in children.iter().enumerate() {
                    let child_path = format!("{path}/{keyword}/{index}");
                    rewritten.push(self.node(child, &child_path, depth + 1, false)?);
                }
                out.insert(keyword.into(), Value::Array(rewritten));
            }
        }
        match out.get("items").cloned() {
            Some(items @ Value::Object(_)) => {
                let rewritten = self.node(&items, &format!("{path}/items"), depth + 1, false)?;
                out.insert("items".into(), rewritten);
            }
            Some(Value::Array(children)) => {
                let mut rewritten = Vec::with_capacity(children.len());
                for (index, child) in children.iter().enumerate() {
                    let child_path = format!("{path}/items/{index}");
                    rewritten.push(self.node(child, &child_path, depth + 1, false)?);
                }
                out.insert("items".into(), Value::Array(rewritten));
            }
            _ => {}
        }

        Ok(Value::Object(out))
    }
}

/// Pass two: walk the rewritten schema **following references**, so a shared
/// definition is counted at every use (the conservative reading of the API's
/// limits), and a reference that leads back to itself is found.
struct Counter<'a> {
    root: &'a Value,
    nodes: usize,
    optional: usize,
    unions: usize,
}

impl Counter<'_> {
    fn node(&mut self, schema: &Value, depth: usize, stack: &mut Vec<String>) -> Result<(), SchemaError> {
        let Some(map) = schema.as_object() else {
            return Ok(());
        };
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(SchemaError::TooLarge { limit: MAX_NODES });
        }
        if depth > MAX_DEPTH {
            return Err(SchemaError::TooDeep {
                path: String::from("(through references)"),
            });
        }

        if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
            let Some((folder, name)) = local_definition(reference) else {
                return Err(SchemaError::UnknownRef {
                    reference: reference.to_string(),
                });
            };
            let Some(definition) = self.root.get(folder).and_then(|defs| defs.get(name)) else {
                return Err(SchemaError::UnknownRef {
                    reference: reference.to_string(),
                });
            };
            if stack.iter().any(|open| open == name) {
                return Err(SchemaError::Recursive {
                    definition: name.to_string(),
                });
            }
            stack.push(name.to_string());
            let definition = definition.clone();
            let result = self.node(&definition, depth + 1, stack);
            stack.pop();
            return result;
        }

        let is_union = map.get("anyOf").is_some_and(Value::is_array)
            || map.get("oneOf").is_some_and(Value::is_array)
            || map
                .get("type")
                .and_then(Value::as_array)
                .is_some_and(|types| types.len() > 1);
        if is_union {
            self.unions += 1;
        }

        if let Some(Value::Object(properties)) = map.get("properties") {
            let required: Vec<&str> = map
                .get("required")
                .and_then(Value::as_array)
                .map(|names| names.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            for (name, child) in properties {
                if !required.contains(&name.as_str()) {
                    self.optional += 1;
                }
                self.node(child, depth + 1, stack)?;
            }
        }
        for keyword in ["anyOf", "oneOf", "allOf", "prefixItems"] {
            if let Some(Value::Array(children)) = map.get(keyword) {
                for child in children {
                    self.node(child, depth + 1, stack)?;
                }
            }
        }
        match map.get("items") {
            Some(items @ Value::Object(_)) => self.node(items, depth + 1, stack)?,
            Some(Value::Array(children)) => {
                for child in children {
                    self.node(child, depth + 1, stack)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
