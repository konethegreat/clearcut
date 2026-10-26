// Written by Kone Tshivhinda & Claude
use super::*;
use crate::tests::{send, FakeAuth};
use crate::{finish, routes, AppState};
use axum::body::Body;
use axum::http::{Request, StatusCode};

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../contracts/openapi.json");

fn rendered() -> String {
    let mut text = serde_json::to_string_pretty(&document()).unwrap();
    text.push('\n');
    text
}

#[test]
fn the_checked_in_openapi_file_is_what_the_code_describes() {
    if std::env::var("CLEARCUT_UPDATE_CONTRACTS").is_ok() {
        std::fs::write(FILE, rendered()).unwrap();
        return;
    }
    let on_disk = std::fs::read_to_string(FILE)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        on_disk == rendered(),
        "contracts/openapi.json is stale: run `CLEARCUT_UPDATE_CONTRACTS=1 cargo test -p clearcut-server openapi` and commit it"
    );
}

#[tokio::test]
async fn every_documented_route_exists_and_every_protected_one_asks_for_a_key() {
    let app = finish(routes(), AppState::shared(FakeAuth::new()));
    let doc = document();
    let paths = doc["paths"].as_object().unwrap();
    assert!(paths.len() >= 12);
    for (path, methods) in paths {
        let concrete = path
            .replace("{id}", "00000000-0000-0000-0000-000000000d0c")
            .replace("{page}", "1");
        for (method, operation) in methods.as_object().unwrap() {
            let request = Request::builder()
                .method(method.to_uppercase().as_str())
                .uri(&concrete)
                .body(Body::empty())
                .unwrap();
            let status = send(&app, request).await.0;
            assert!(
                status != StatusCode::NOT_FOUND && status != StatusCode::METHOD_NOT_ALLOWED,
                "{method} {path} is documented but the server does not know it ({status})"
            );
            // Every call but the health check refuses a caller with no key (the answer comes before the body is read).
            if path != "/health" {
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}: {operation}");
            }
        }
    }
}

#[test]
fn every_reference_in_the_document_resolves_inside_it() {
    fn walk(value: &Value, root: &Value, found: &mut usize) {
        match value {
            Value::Object(map) => {
                if let Some(Value::String(reference)) = map.get("$ref") {
                    *found += 1;
                    let target = reference.strip_prefix("#/").expect("a local reference");
                    let resolved = target.split('/').try_fold(root, |at, key| at.get(key));
                    assert!(resolved.is_some(), "{reference} points at nothing");
                }
                map.values().for_each(|v| walk(v, root, found));
            }
            Value::Array(items) => items.iter().for_each(|v| walk(v, root, found)),
            _ => {}
        }
    }
    let doc = document();
    let mut found = 0;
    walk(&doc, &doc, &mut found);
    assert!(found >= 8, "only {found} references");
}
