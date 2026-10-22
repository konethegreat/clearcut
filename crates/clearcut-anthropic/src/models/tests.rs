// Written by Kone Tshivhinda & Claude
use super::*;
use crate::config::AnthropicConfig;
use crate::testutil::{Answer, Stub, TEST_KEY};
use serde_json::json;
use std::time::Duration;

fn http_for(stub: &Stub) -> AnthropicHttp {
    AnthropicHttp::new(
        &AnthropicConfig::new(&stub.url).unwrap(),
        &ApiKey::parse(TEST_KEY).unwrap(),
    )
    .unwrap()
}

fn page(models: serde_json::Value, has_more: bool, last_id: Option<&str>) -> String {
    json!({ "data": models, "has_more": has_more, "first_id": null, "last_id": last_id }).to_string()
}

fn opus() -> serde_json::Value {
    json!({
        "type": "model",
        "id": "claude-opus-5-5",
        "display_name": "Claude Opus 5.5",
        "created_at": "2026-09-21T00:00:00Z",
        "lifecycle": "active",
        "deprecated_at": null,
        "retires_at": null,
        "line": "opus",
        "max_input_tokens": 1000000,
        "max_tokens": 128000,
        "capabilities": {
            "batch": { "supported": true },
            "image_input": { "supported": true },
            "pdf_input": { "supported": true },
            "structured_outputs": { "supported": true },
            "effort": { "supported": true, "low": { "supported": true } },
            "something_new": { "supported": true }
        },
        "a_field_we_do_not_know": [1, 2, 3]
    })
}

fn bare(id: &str) -> serde_json::Value {
    json!({ "id": id, "display_name": id, "lifecycle": "active" })
}

async fn check(answers: Vec<Answer>) -> (Result<KeyCheck, KeyCheckError>, Stub) {
    let stub = Stub::start(answers).await;
    let result = check_key(&http_for(&stub)).await;
    (result, stub)
}

fn kind(result: Result<KeyCheck, KeyCheckError>) -> KeyCheckKind {
    result.expect_err("an error").kind()
}

// ---- a good key ----

#[tokio::test]
async fn a_good_key_gives_the_models_it_can_use() {
    let (result, stub) = check(vec![Answer::json(
        200,
        &page(json!([opus(), bare("claude-haiku-5-5")]), false, None),
    )
    .with_header("request-id", "req_011CSHoEeqs5C35K2UUqR7Fy")])
    .await;
    let check = result.unwrap();
    assert_eq!(check.request_id.as_deref(), Some("req_011CSHoEeqs5C35K2UUqR7Fy"));
    assert_eq!(check.models.len(), 2);

    let model = check.model("claude-opus-5-5").unwrap();
    assert_eq!(model.display_name, "Claude Opus 5.5");
    assert_eq!(model.lifecycle, Lifecycle::Active);
    assert_eq!(model.max_input_tokens, Some(1_000_000));
    assert_eq!(model.max_tokens, Some(128_000));
    assert!(model.structured_outputs && model.image_input && model.pdf_input && model.batch);

    // A model that says nothing about capabilities supports nothing, not everything.
    let plain = check.model("claude-haiku-5-5").unwrap();
    assert!(!plain.structured_outputs && !plain.image_input && !plain.pdf_input && !plain.batch);
    assert_eq!(plain.max_tokens, None);

    assert!(check.model("claude-nonexistent").is_none());
    let seen = &stub.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/v1/models");
    assert_eq!(seen[0].query.as_deref(), Some("limit=1000"));
}

#[tokio::test]
async fn only_models_that_can_still_be_called_are_usable() {
    let models = json!([
        { "id": "a-active", "display_name": "a", "lifecycle": "active" },
        { "id": "b-deprecated", "display_name": "b", "lifecycle": "deprecated" },
        { "id": "c-retired", "display_name": "c", "lifecycle": "retired" },
        { "id": "d-future", "display_name": "d", "lifecycle": "something-new" },
        { "id": "e-missing", "display_name": "e" },
    ]);
    let check = check(vec![Answer::json(200, &page(models, false, None))])
        .await
        .0
        .unwrap();
    assert_eq!(check.model("d-future").unwrap().lifecycle, Lifecycle::Unknown);
    assert_eq!(check.model("e-missing").unwrap().lifecycle, Lifecycle::Unknown);
    let usable: Vec<&str> = [
        "a-active",
        "b-deprecated",
        "c-retired",
        "d-future",
        "e-missing",
        "zzz",
    ]
    .into_iter()
    .filter(|id| check.can_use(id))
    .collect();
    assert_eq!(usable, ["a-active", "b-deprecated"]);
}

#[tokio::test]
async fn an_empty_list_is_a_valid_answer() {
    let check = check(vec![Answer::json(200, &page(json!([]), false, None))])
        .await
        .0
        .unwrap();
    assert!(check.models.is_empty());
}

// ---- several pages ----

#[tokio::test]
async fn further_pages_are_fetched_until_the_list_ends() {
    let (result, stub) = check(vec![
        Answer::json(200, &page(json!([bare("m1"), bare("m2")]), true, Some("m2"))),
        Answer::json(200, &page(json!([bare("m3")]), false, Some("m3"))),
    ])
    .await;
    let ids: Vec<String> = result.unwrap().models.into_iter().map(|m| m.id).collect();
    assert_eq!(ids, ["m1", "m2", "m3"]);
    let seen = stub.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].query.as_deref(), Some("limit=1000&after_id=m2"));
}

#[tokio::test]
async fn a_list_that_never_ends_is_stopped() {
    let answers: Vec<Answer> = (0..10)
        .map(|n| {
            Answer::json(
                200,
                &page(json!([bare(&format!("m{n}"))]), true, Some(&format!("m{n}"))),
            )
        })
        .collect();
    let (result, stub) = check(answers).await;
    assert_eq!(kind(result), KeyCheckKind::TooManyPages);
    assert_eq!(MAX_PAGES, 5);
    assert_eq!(stub.requests().len(), 5);
}

#[tokio::test]
async fn a_cursor_that_does_not_move_is_a_bad_answer() {
    let (result, stub) = check(vec![Answer::json(
        200,
        &page(json!([bare("m1")]), true, Some("m1")),
    )])
    .await;
    assert_eq!(kind(result), KeyCheckKind::BadResponse);
    assert_eq!(stub.requests().len(), 2);
}

#[tokio::test]
async fn a_cursor_with_odd_characters_is_never_sent_back() {
    for cursor in ["a b", "a&b=c", "x/y", "é", "", &"a".repeat(129)] {
        let (result, stub) = check(vec![Answer::json(
            200,
            &page(json!([bare("m1")]), true, Some(cursor)),
        )])
        .await;
        assert_eq!(kind(result), KeyCheckKind::BadResponse, "{cursor:?}");
        assert_eq!(stub.requests().len(), 1, "{cursor:?}");
    }
    // More to come but no cursor.
    let (result, _) = check(vec![Answer::json(200, &page(json!([bare("m1")]), true, None))]).await;
    assert_eq!(kind(result), KeyCheckKind::BadResponse);
}

// ---- failures, each mapped to what the key's owner should do ----

#[tokio::test]
async fn every_status_maps_to_one_kind() {
    let cases: Vec<(u16, &str, KeyCheckKind)> = vec![
        (
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"x"}}"#,
            KeyCheckKind::Rejected,
        ),
        (402, "{}", KeyCheckKind::BillingProblem),
        (403, "{}", KeyCheckKind::NoPermission),
        (404, "{}", KeyCheckKind::Unexpected { status: 404 }),
        (400, "{}", KeyCheckKind::Unexpected { status: 400 }),
        (413, "{}", KeyCheckKind::Unexpected { status: 413 }),
        (500, "{}", KeyCheckKind::Unavailable { status: 500 }),
        (502, "{}", KeyCheckKind::Unavailable { status: 502 }),
        (504, "{}", KeyCheckKind::Unavailable { status: 504 }),
        (
            529,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"x"}}"#,
            KeyCheckKind::Unavailable { status: 529 },
        ),
        (302, "", KeyCheckKind::Unexpected { status: 302 }),
        (201, "{}", KeyCheckKind::Unexpected { status: 201 }),
    ];
    for (status, body, expected) in cases {
        let (result, _) = check(vec![Answer::json(status, body)]).await;
        assert_eq!(kind(result), expected, "{status}");
    }
}

#[tokio::test]
async fn a_rate_limit_carries_how_long_to_wait() {
    let body = r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#;
    let (result, _) = check(vec![Answer::json(429, body).with_header("retry-after", "17")]).await;
    assert_eq!(
        kind(result),
        KeyCheckKind::RateLimited {
            retry_after_secs: Some(17)
        }
    );
    let (result, _) = check(vec![Answer::json(429, body)]).await;
    assert_eq!(
        kind(result),
        KeyCheckKind::RateLimited {
            retry_after_secs: None
        }
    );
    let (result, _) = check(vec![Answer::json(429, "not json")]).await;
    assert_eq!(
        kind(result),
        KeyCheckKind::RateLimited {
            retry_after_secs: None
        }
    );
}

#[tokio::test]
async fn the_monthly_spend_cap_is_told_apart_from_a_rate_limit() {
    let body = r#"{"type":"error","error":{"type":"rate_limit_error","message":"You have reached your API usage limits",
        "details":{"error_code":"enforced_spend_limit_reached"}},"request_id":"req_x"}"#;
    // It has no retry-after, and waiting a minute would not help.
    let (result, _) = check(vec![Answer::json(429, body)]).await;
    assert_eq!(kind(result), KeyCheckKind::SpendCapReached);
    // Even if a header were present, the cap is the cap.
    let (result, _) = check(vec![Answer::json(429, body).with_header("retry-after", "5")]).await;
    assert_eq!(kind(result), KeyCheckKind::SpendCapReached);
}

#[tokio::test]
async fn nothing_the_server_wrote_is_repeated() {
    let (result, _) = check(vec![Answer::json(
        401,
        r#"{"type":"error","error":{"type":"authentication_error","message":"SECRET-SERVER-TEXT"}}"#,
    )
    .with_header("request-id", "req_abc123")])
    .await;
    let error = result.unwrap_err();
    let shown = format!("{error} {error:?}");
    assert!(!shown.contains("SECRET-SERVER-TEXT"), "{shown}");
    assert!(
        error.to_string().contains("req_abc123"),
        "the request id is for support: {error}"
    );
    assert_eq!(error.request_id(), Some("req_abc123"));
}

#[tokio::test]
async fn an_answer_that_is_not_a_model_list_is_a_bad_response() {
    for body in [
        "",
        "not json",
        "[]",
        "{}",
        r#"{"data":"nope"}"#,
        r#"{"data":[{"display_name":"no id"}]}"#,
        r#"{"data":[{"id":5}]}"#,
        r#"{"data":[],"has_more":"yes"}"#,
    ] {
        let (result, _) = check(vec![Answer::json(200, body)]).await;
        assert_eq!(kind(result), KeyCheckKind::BadResponse, "{body:?}");
    }
}

#[tokio::test]
async fn a_huge_answer_is_refused() {
    let big = format!(r#"{{"data":[],"padding":"{}"}}"#, "x".repeat(MAX_PAGE_BYTES + 10));
    let (result, _) = check(vec![Answer::json(200, &big)]).await;
    assert_eq!(kind(result), KeyCheckKind::BadResponse);
}

#[tokio::test]
async fn network_trouble_is_reported_as_such() {
    let stub = Stub::start(vec![Answer::json(200, "{}").after(Duration::from_millis(600))]).await;
    let config = AnthropicConfig::new(&stub.url)
        .unwrap()
        .with_timeout(Duration::from_millis(100))
        .unwrap();
    let http = AnthropicHttp::new(&config, &ApiKey::parse(TEST_KEY).unwrap()).unwrap();
    assert_eq!(kind(check_key(&http).await), KeyCheckKind::Timeout);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let http = AnthropicHttp::new(
        &AnthropicConfig::new(&url).unwrap(),
        &ApiKey::parse(TEST_KEY).unwrap(),
    )
    .unwrap();
    assert_eq!(kind(check_key(&http).await), KeyCheckKind::Unreachable);
}

#[tokio::test]
async fn each_kind_says_what_the_owner_of_the_key_should_do() {
    for (kind, words) in [
        (KeyCheckKind::Rejected, "rejected"),
        (KeyCheckKind::NoPermission, "permission"),
        (KeyCheckKind::BillingProblem, "billing"),
        (
            KeyCheckKind::RateLimited {
                retry_after_secs: Some(9),
            },
            "9 seconds",
        ),
        (
            KeyCheckKind::RateLimited {
                retry_after_secs: None,
            },
            "rate limit",
        ),
        (KeyCheckKind::SpendCapReached, "monthly"),
        (KeyCheckKind::Unavailable { status: 529 }, "529"),
        (KeyCheckKind::Unexpected { status: 418 }, "418"),
        (KeyCheckKind::Unreachable, "reach"),
        (KeyCheckKind::Timeout, "too long"),
        (KeyCheckKind::Interrupted, "cut"),
        (KeyCheckKind::BadResponse, "understood"),
        (KeyCheckKind::TooManyPages, "pages"),
    ] {
        let error = KeyCheckError {
            kind,
            request_id: None,
        };
        assert!(error.to_string().contains(words), "{error}");
    }
}

#[test]
fn only_the_keys_own_problems_mean_the_owner_must_act() {
    use KeyCheckKind::*;
    for (kind, owner_must_act) in [
        (Rejected, true),
        (NoPermission, true),
        (BillingProblem, true),
        (SpendCapReached, true),
        (
            RateLimited {
                retry_after_secs: Some(1),
            },
            false,
        ),
        (Unavailable { status: 500 }, false),
        (Unexpected { status: 400 }, false),
        (Unreachable, false),
        (Timeout, false),
        (Interrupted, false),
        (BadResponse, false),
        (TooManyPages, false),
    ] {
        assert_eq!(kind.owner_must_act(), owner_must_act, "{kind:?}");
    }
}

#[tokio::test]
async fn every_server_error_status_is_trouble_on_the_way() {
    for status in [500u16, 503, 598, 599] {
        let (result, _) = check(vec![Answer::json(status, "{}")]).await;
        assert_eq!(kind(result), KeyCheckKind::Unavailable { status }, "{status}");
    }
}

#[tokio::test]
async fn a_page_that_does_not_say_whether_there_is_more_is_the_last() {
    let (result, stub) = check(vec![Answer::json(200, r#"{"data":[{"id":"m1"}]}"#)]).await;
    assert_eq!(result.unwrap().models.len(), 1);
    assert_eq!(stub.requests().len(), 1);
}

#[tokio::test]
async fn a_capability_that_is_present_but_silent_counts_as_unsupported() {
    let models = json!([{ "id": "m1", "capabilities": { "image_input": {}, "pdf_input": { "supported": null }, "batch": { "supported": false } } }]);
    let check = check(vec![Answer::json(200, &page(models, false, None))])
        .await
        .0
        .unwrap();
    let model = check.model("m1").unwrap();
    assert!(!model.image_input && !model.pdf_input && !model.batch && !model.structured_outputs);
}

#[tokio::test]
async fn each_capability_is_read_from_its_own_field() {
    let only = |name: &str| {
        let mut capabilities = serde_json::Map::new();
        capabilities.insert(name.to_string(), json!({ "supported": true }));
        json!([{ "id": "m1", "capabilities": capabilities }])
    };
    for (name, expected) in [
        ("structured_outputs", (true, false, false, false)),
        ("image_input", (false, true, false, false)),
        ("pdf_input", (false, false, true, false)),
        ("batch", (false, false, false, true)),
    ] {
        let check = check(vec![Answer::json(200, &page(only(name), false, None))])
            .await
            .0
            .unwrap();
        let m = check.model("m1").unwrap();
        assert_eq!(
            (m.structured_outputs, m.image_input, m.pdf_input, m.batch),
            expected,
            "{name}"
        );
    }
}

#[tokio::test]
async fn a_rate_limit_without_the_cap_code_is_still_just_a_rate_limit() {
    let body =
        r#"{"type":"error","error":{"type":"rate_limit_error","details":{"error_code":"something_else"}}}"#;
    let (result, _) = check(vec![Answer::json(429, body).with_header("retry-after", "3")]).await;
    assert_eq!(
        kind(result),
        KeyCheckKind::RateLimited {
            retry_after_secs: Some(3)
        }
    );
}
