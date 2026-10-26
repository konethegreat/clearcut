// Written by Kone Tshivhinda & Claude
//! The REST API described as an OpenAPI 3.1 document: `contracts/openapi.json` in the repository.
//!
//! It is **generated from this code** and a test fails when the checked-in file is stale, so it
//! cannot drift. The shapes of the core types (`Invoice`, `InvoiceTotals`, `PageReading`) come from
//! the types themselves through `schemars`; the route list below is written by hand, and a second
//! test sends a request to every documented route and fails if the server does not know it.
//!
//! To rewrite the file after changing the API:
//! `CLEARCUT_UPDATE_CONTRACTS=1 cargo test -p clearcut-server openapi`.
//!
//! The promise to callers (docs/integration/timelex.md): everything under `/v1/` only gains
//! optional fields; a breaking change is a new `/v2/` path.

use clearcut_core::reading::reading_schema;
use clearcut_core::{Invoice, InvoiceTotals};
use serde_json::{json, Value};

fn schema_of<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null)
}

/// A reference to a reusable error body, repeated on every protected call.
fn problem(status: &str, text: &str) -> (String, Value) {
    (
        status.to_string(),
        json!({ "description": text, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } }),
    )
}

fn responses(ok: Value, others: &[(&str, &str)]) -> Value {
    let mut map = serde_json::Map::new();
    for (code, body) in ok.as_object().cloned().unwrap_or_default() {
        map.insert(code, body);
    }
    for (code, text) in others {
        let (code, body) = problem(code, text);
        map.insert(code, body);
    }
    Value::Object(map)
}

fn ok_json(status: &str, description: &str, schema: Value) -> Value {
    json!({ status: { "description": description, "content": { "application/json": { "schema": schema } } } })
}

fn id_param() -> Value {
    json!({ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } })
}

/// One operation. `scope` is the scope the call needs (`None` for a call that needs only a valid key).
fn op(
    summary: &str,
    scope: Option<&str>,
    parameters: Vec<Value>,
    request: Option<Value>,
    responses: Value,
) -> Value {
    let mut operation = json!({ "summary": summary, "parameters": parameters, "responses": responses });
    if let Some(scope) = scope {
        operation["x-required-scope"] = json!(scope);
    }
    if let Some(request) = request {
        operation["requestBody"] = request;
    }
    operation
}

/// The whole document.
pub fn document() -> Value {
    let mut invoice = schema_of::<Invoice>();
    let totals = schema_of::<InvoiceTotals>();
    // The core types' own definitions are inlined as components so references resolve in one file.
    let mut schemas = serde_json::Map::new();
    for schema in [&mut invoice] {
        if let Some(defs) = schema.get("$defs").and_then(Value::as_object).cloned() {
            for (name, def) in defs {
                schemas.insert(name, def);
            }
        }
        if let Some(object) = schema.as_object_mut() {
            object.remove("$defs");
            object.remove("$schema");
        }
    }
    let mut totals = totals;
    if let Some(defs) = totals.get("$defs").and_then(Value::as_object).cloned() {
        for (name, def) in defs {
            schemas.insert(name, def);
        }
    }
    if let Some(object) = totals.as_object_mut() {
        object.remove("$defs");
        object.remove("$schema");
    }
    let mut reading = serde_json::to_value(reading_schema()).unwrap_or(Value::Null);
    if let Some(object) = reading.as_object_mut() {
        object.remove("$schema");
    }
    schemas.insert("Invoice".into(), invoice);
    schemas.insert("InvoiceTotals".into(), totals);
    schemas.insert("PageReading".into(), reading);
    schemas.insert(
        "Error".into(),
        json!({ "type": "object", "required": ["error"], "properties": { "error": { "type": "string" } } }),
    );
    schemas.insert(
        "Refusal".into(),
        json!({
            "description": "Why a page cannot be confirmed yet: fixed names and places, never text from the document.",
            "type": "object",
            "properties": {
                "currency_missing": { "type": "boolean" },
                "problems": { "type": "array", "items": { "type": "object", "properties": { "path": { "type": "string" }, "kind": { "type": "string" } } } },
                "findings": { "type": "array", "items": { "type": "object", "description": "A check that failed; has a `type` and, for some, amounts." } }
            }
        }),
    );

    let doc = |id: &str| -> Vec<Value> {
        let _ = id;
        vec![id_param()]
    };
    let auth = [
        ("401", "No valid key (the answer is identical for every failure)"),
        ("403", "The key lacks the required scope"),
    ];
    let with_auth = |extra: &[(&'static str, &'static str)]| -> Vec<(&'static str, &'static str)> {
        let mut all: Vec<(&str, &str)> = auth.to_vec();
        all.extend_from_slice(extra);
        all
    };

    let paths = json!({
        "/health": { "get": op("Is the server up (no key needed)", None, vec![], None,
            ok_json("200", "Up", json!({ "type": "object", "properties": { "status": { "type": "string" }, "version": { "type": "string" } } }))) },
        "/v1/whoami": { "get": op("Which tenant, key and scopes these credentials carry", None, vec![], None,
            responses(ok_json("200", "The caller", json!({ "type": "object", "properties": { "tenant": { "type": "string" }, "key_id": { "type": "string" }, "scopes": { "type": "array", "items": { "type": "string" } } } })), &auth)) },
        "/v1/documents": {
            "post": op("Upload one document (the request body is the file itself)", Some("documents:write"),
                vec![json!({ "name": "filename", "in": "query", "required": false, "schema": { "type": "string" } })],
                Some(json!({ "required": true, "content": {
                    "application/pdf": { "schema": { "type": "string", "format": "binary" } },
                    "image/png": { "schema": { "type": "string", "format": "binary" } },
                    "image/jpeg": { "schema": { "type": "string", "format": "binary" } } } })),
                responses(json!({
                    "201": { "description": "Stored and queued to be read", "content": { "application/json": { "schema": { "type": "object", "properties": { "document_id": { "type": "string" }, "job_id": { "type": ["string", "null"] }, "duplicate": { "type": "boolean" }, "sha256": { "type": "string" }, "byte_size": { "type": "integer" }, "filename": { "type": "string" }, "media_type": { "type": "string" } } } } } },
                    "200": { "description": "This tenant had uploaded exactly these bytes before; the existing document is returned" } }),
                    &with_auth(&[("413", "Over the size limit"), ("415", "Not a PDF, PNG, JPEG, TIFF or XML")]))),
            "get": op("The tenant's most recent documents, newest first", Some("documents:read"), vec![], None,
                responses(ok_json("200", "A list", json!({ "type": "object", "properties": { "documents": { "type": "array", "items": { "type": "object" } } } })), &auth)),
        },
        "/v1/documents/{id}": {
            "get": op("A document's status, page count, file name and number of events", Some("documents:read"), doc("id"), None,
                responses(ok_json("200", "The document", json!({ "type": "object", "properties": { "id": { "type": "string" }, "status": { "type": "string", "enum": ["received", "processing", "needs_review", "done"] }, "page_count": { "type": ["integer", "null"] }, "original_filename": { "type": "string" }, "events": { "type": "integer" } } })), &with_auth(&[("404", "No such document, or it belongs to another tenant")]))),
            "delete": op("Erase the document, everything read from it and its file for good (POPIA)", Some("documents:write"), doc("id"), None,
                responses(json!({ "204": { "description": "Gone" } }), &with_auth(&[("404", "No such document, or it belongs to another tenant"), ("409", "A job is running on it; try again when it has finished")]))),
        },
        "/v1/documents/{id}/events": { "get": op("The events as stored: the source of truth", Some("documents:read"), doc("id"), None,
            responses(ok_json("200", "Events and a one-line-each table", json!({ "type": "object", "properties": { "events": { "type": "array", "items": { "type": "object" } }, "table": { "type": "array", "items": { "type": "object" } } } })), &with_auth(&[("404", "Not found")]))) },
        "/v1/documents/{id}/report": { "get": op("The cited narration; every sentence names the events it comes from", Some("documents:read"),
            vec![id_param(), json!({ "name": "mode", "in": "query", "schema": { "type": "string", "enum": ["summary", "timeline", "grouped"] } })], None,
            responses(ok_json("200", "A report", json!({ "type": "object" })), &with_auth(&[("400", "Unknown mode"), ("404", "Not found")]))) },
        "/v1/documents/{id}/export": { "get": op("The events as a file to download", Some("documents:read"),
            vec![id_param(), json!({ "name": "format", "in": "query", "schema": { "type": "string", "enum": ["json", "csv", "md"] } })], None,
            responses(json!({ "200": { "description": "The file", "content": { "application/json": {}, "text/csv": {}, "text/markdown": {} } } }), &with_auth(&[("400", "Unknown format"), ("404", "Not found")]))) },
        "/v1/documents/{id}/jobs": { "get": op("What is happening to a document: its jobs, newest first", Some("documents:read"), doc("id"), None,
            responses(ok_json("200", "Jobs", json!({ "type": "object", "properties": { "jobs": { "type": "array", "items": { "type": "object" } } } })), &with_auth(&[("404", "Not found")]))) },
        "/v1/documents/{id}/review": { "get": op("The pages waiting for a person, each with the reason it stopped and the reader's best draft", Some("documents:read"), doc("id"), None,
            responses(ok_json("200", "Pages", json!({ "type": "object", "properties": { "pages": { "type": "array", "items": { "type": "object", "properties": { "page": { "type": "integer" }, "reason": { "type": "string" }, "problems": { "type": "array" }, "findings": { "type": "array" }, "reading": { "oneOf": [{ "$ref": "#/components/schemas/PageReading" }, { "type": "null" }] } } } } } })), &with_auth(&[("404", "Not found")]))) },
        "/v1/documents/{id}/pages/{page}/review": { "post": op("A person's reading of a page, checked by the same rules as a model's; if it passes the page is confirmed and the document finished with no model call", Some("documents:write"),
            vec![id_param(), json!({ "name": "page", "in": "path", "required": true, "schema": { "type": "integer", "minimum": 1 } })],
            Some(json!({ "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/PageReading" } } } })),
            responses(ok_json("200", "Confirmed; the document is queued to be finished", json!({ "type": "object", "properties": { "job_id": { "type": "string" } } })),
                &with_auth(&[("400", "Not a complete reading"), ("404", "Not found"), ("409", "The page is not waiting for review, or the document is being worked on"), ("422", "Not good enough yet: see `refusal`")]))) },
        "/v1/jobs/{id}/cancel": { "post": op("Cancel a job (a running one is asked to stop)", Some("documents:write"), vec![id_param()], None,
            responses(json!({ "200": { "description": "Cancelled, or already finished" }, "202": { "description": "Running; it has been asked to stop" } }), &with_auth(&[("404", "Not found")]))) },
        "/v1/anthropic-key": {
            "get": op("Whether this tenant has its own Anthropic key (never the key), and whose key the next job would use", Some("keys:admin"), vec![], None,
                responses(ok_json("200", "Status", json!({ "type": "object", "properties": { "configured": { "type": "boolean" }, "last4": { "type": ["string", "null"] }, "set_at": { "type": ["string", "null"] }, "checked_at": { "type": ["string", "null"] }, "check_ok": { "type": ["boolean", "null"] }, "source": { "type": "string", "enum": ["tenant", "server", "none"] }, "mode": { "type": "string", "enum": ["shared", "tenant-only"] }, "tenant_keys_enabled": { "type": "boolean" } } })), &auth)),
            "put": op("Set the tenant's own Anthropic key: checked with Anthropic for free, then stored encrypted; never shown again", Some("keys:admin"), vec![],
                Some(json!({ "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["key"], "properties": { "key": { "type": "string" } }, "additionalProperties": false } } } })),
                responses(ok_json("200", "Stored", json!({ "type": "object", "properties": { "last4": { "type": "string" }, "models": { "type": "array", "items": { "type": "string" } }, "checked_at": { "type": "string" } } })),
                    &with_auth(&[("400", "Not a JSON object with one text field called key"), ("422", "An admin key, a login token, or a key Anthropic does not accept"), ("502", "Anthropic could not be reached to check it; nothing was stored"), ("503", "The server has no master key, so tenant keys are off")]))),
            "delete": op("Remove the tenant's own Anthropic key", Some("keys:admin"), vec![], None,
                responses(ok_json("200", "Whether there was one", json!({ "type": "object", "properties": { "removed": { "type": "boolean" } } })), &auth)),
        },
        "/v1/invoices/calculate": { "post": op("Net, VAT and gross of an invoice: pure arithmetic, nothing stored. VAT is taken per tax category on the category subtotal. A line may carry `line_total`, used as given.", None, vec![],
            Some(json!({ "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Invoice" } } } })),
            responses(ok_json("200", "Totals in integer minor units", json!({ "type": "object", "properties": { "totals": { "$ref": "#/components/schemas/InvoiceTotals" }, "required_document": { "type": "string" }, "display_gross": { "type": "string" } } })), &[("401", "No valid key"), ("422", "The invoice cannot be calculated (no lines, a bad quantity, mixed currencies)")])) },
    });

    // The core types were generated with their own `#/$defs/...` references; their definitions now
    // live under `components`, so the references must point there.
    let document = json!({
        "openapi": "3.1.0",
        "info": {
            "title": "ClearCut API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Upload an invoice, have it read and checked, get cited events, narration and exports. Money is integer minor units (cents), quantities are thousandths. Everything under /v1/ only gains optional fields; a breaking change is a new /v2/ path. Generated from the code; do not edit by hand.",
            "license": { "name": "Apache-2.0", "identifier": "Apache-2.0" }
        },
        "servers": [{ "url": "http://127.0.0.1:8080", "description": "A ClearCut server on your own computer (the port is CLEARCUT_BIND)" }],
        "security": [{ "bearer": [] }],
        "paths": paths,
        "components": {
            "securitySchemes": { "bearer": { "type": "http", "scheme": "bearer", "description": "A ClearCut API key (ccut_...), never an Anthropic key. Created with clearcut-admin or by a key with keys:admin." } },
            "schemas": Value::Object(schemas)
        }
    });
    serde_json::from_str(&document.to_string().replace("#/$defs/", "#/components/schemas/"))
        .unwrap_or(document)
}

#[cfg(test)]
mod tests;
