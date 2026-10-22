// Written by Kone Tshivhinda & Claude
use super::*;
use crate::testutil::{Answer, Stub, TEST_KEY};
use clearcut_core::jobs::FailureKind;
use clearcut_provider::{
    Block, CallContext, FakeProvider, KeyProblem, ModelProvider, ModelRequest, ModelResponse, Purpose,
    Thinking, Usage,
};
use serde_json::{json, Value};
use std::sync::Mutex;
use uuid::Uuid;

const NOW: u64 = 1_792_670_400; // 2026-10-22 12:00:00 UTC
const SECRET: &str = "ACCOUNT 1234567890 SECRET";

/// Records the waits instead of waiting, so no test sleeps, and fixes the
/// jitter and the clock so every number is exact.
#[derive(Default)]
struct TestPacing {
    pauses: Mutex<Vec<Duration>>,
}

impl TestPacing {
    fn pauses(&self) -> Vec<Duration> {
        self.pauses.lock().unwrap().clone()
    }
}

impl Pacing for TestPacing {
    fn pause(&self, wait: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        self.pauses.lock().unwrap().push(wait);
        Box::pin(async {})
    }

    fn jitter(&self, up_to: Duration) -> Duration {
        up_to / 2
    }

    fn now_unix(&self) -> u64 {
        NOW
    }
}

fn provider_for(stub: &Stub) -> (AnthropicProvider, Arc<TestPacing>) {
    let pacing = Arc::new(TestPacing::default());
    let config = AnthropicConfig::new(&stub.url).unwrap();
    let key = ApiKey::parse(TEST_KEY).unwrap();
    let provider = AnthropicProvider::new(&config, &key)
        .unwrap()
        .with_pacing(pacing.clone());
    (provider, pacing)
}

fn request() -> ModelRequest {
    ModelRequest::builder(
        "claude-haiku-5-5",
        CallContext {
            tenant: Uuid::from_u128(1),
            job: None,
            document: None,
            page: Some(1),
            purpose: Purpose::Extraction,
        },
    )
    .cached_system("Rules.")
    .content(Block::text(SECRET))
    .output_schema(json!({
        "type": "object",
        "properties": { "invoice_number": { "type": "string" } },
        "required": ["invoice_number"]
    }))
    .max_output_tokens(2000)
    .build()
    .unwrap()
}

fn good_answer() -> Answer {
    Answer::json(
        200,
        &json!({
            "id": "msg_1", "type": "message", "role": "assistant",
            "model": "claude-haiku-5-5",
            "content": [{ "type": "text", "text": "{\"invoice_number\":\"INV-1\"}" }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 100, "output_tokens": 20,
                       "cache_creation_input_tokens": 50, "cache_read_input_tokens": 5 }
        })
        .to_string(),
    )
    .with_header("request-id", "req_ok")
}

fn error_answer(status: u16, kind: &str) -> Answer {
    Answer::json(
        status,
        &json!({ "type": "error", "error": { "type": kind, "message": SECRET } }).to_string(),
    )
}

fn sec(n: u64) -> Duration {
    Duration::from_secs(n)
}

// ---------- the happy path ----------

#[tokio::test]
async fn a_call_posts_the_built_request_and_returns_the_parsed_answer() {
    let stub = Stub::start(vec![good_answer()]).await;
    let (provider, pacing) = provider_for(&stub);
    let response = provider.complete(request()).await.expect("should succeed");

    assert_eq!(response.output, json!({ "invoice_number": "INV-1" }));
    assert_eq!(response.model, "claude-haiku-5-5");
    assert_eq!(response.request_id.as_deref(), Some("req_ok"));
    assert_eq!(
        response.usage,
        Usage {
            input_tokens: 100,
            output_tokens: 20,
            cache_read_tokens: 5,
            cache_write_tokens: 50,
            cache_write_1h_tokens: 0,
        }
    );

    let seen = stub.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].path, "/v1/messages");
    assert_eq!(
        seen[0].headers["authorization"],
        format!("Bearer {TEST_KEY}").as_str()
    );
    let sent: Value = serde_json::from_slice(&seen[0].body).unwrap();
    assert_eq!(sent, crate::request::build(&request()).unwrap().body);
    assert!(pacing.pauses().is_empty());
}

#[tokio::test]
async fn the_key_is_never_in_the_body_and_never_printed() {
    let stub = Stub::start(vec![good_answer()]).await;
    let (provider, _) = provider_for(&stub);
    provider.complete(request()).await.unwrap();
    assert!(!String::from_utf8_lossy(&stub.requests()[0].body).contains("sk-ant"));
    let shown = format!("{provider:?}");
    assert!(!shown.contains("sk-ant") && !shown.contains(TEST_KEY), "{shown}");
}

// ---------- refused before anything is sent ----------

#[tokio::test]
async fn a_request_that_cannot_be_built_never_reaches_the_network() {
    let stub = Stub::start(vec![good_answer()]).await;
    let (provider, _) = provider_for(&stub);

    let opus_off = ModelRequest::builder("claude-opus-5-5", request().call().clone())
        .content(Block::text("x"))
        .output_schema(json!({ "type": "object", "properties": {}, "required": [] }))
        .max_output_tokens(500)
        .thinking(Thinking::Off)
        .build()
        .unwrap();
    assert!(matches!(
        provider.complete(opus_off).await,
        Err(ProviderError::Rejected(_))
    ));

    let too_long = ModelRequest::builder("claude-haiku-5-5", request().call().clone())
        .content(Block::text("x"))
        .output_schema(json!({ "type": "object", "properties": {}, "required": [] }))
        .max_output_tokens(30_000)
        .build()
        .unwrap();
    assert!(matches!(
        provider.complete(too_long).await,
        Err(ProviderError::Rejected(_))
    ));
    assert!(stub.requests().is_empty(), "nothing was sent");
}

// ---------- retries ----------

#[tokio::test]
async fn an_outage_is_retried_twice_with_a_growing_wait_and_then_succeeds() {
    let stub = Stub::start(vec![
        error_answer(503, "api_error"),
        error_answer(529, "overloaded_error"),
        good_answer(),
    ])
    .await;
    let (provider, pacing) = provider_for(&stub);
    assert!(provider.complete(request()).await.is_ok());
    assert_eq!(stub.requests().len(), 3);
    // 1 s then 2 s, each plus half of a 500 ms jitter window.
    assert_eq!(
        pacing.pauses(),
        vec![Duration::from_millis(1250), Duration::from_millis(2250)]
    );
}

#[tokio::test]
async fn an_outage_that_lasts_gives_up_after_two_retries() {
    let stub = Stub::start(vec![error_answer(500, "api_error")]).await;
    let (provider, pacing) = provider_for(&stub);
    let error = provider.complete(request()).await.unwrap_err();
    assert!(
        matches!(error, ProviderError::Unavailable { retry_after: None }),
        "{error:?}"
    );
    assert_eq!(stub.requests().len(), 3, "one try and two retries, no more");
    assert_eq!(pacing.pauses().len(), 2);
    assert_eq!(error.failure_kind(), FailureKind::ProviderUnavailable);
}

#[tokio::test]
async fn a_short_rate_limit_is_waited_out_for_exactly_as_long_as_it_asked() {
    let stub = Stub::start(vec![
        error_answer(429, "rate_limit_error").with_header("retry-after", "7"),
        good_answer(),
    ])
    .await;
    let (provider, pacing) = provider_for(&stub);
    assert!(provider.complete(request()).await.is_ok());
    assert_eq!(pacing.pauses(), vec![sec(7)]);
}

#[tokio::test]
async fn two_one_minute_rate_limits_fit_in_the_two_minute_budget() {
    let stub = Stub::start(vec![
        error_answer(429, "rate_limit_error").with_header("retry-after", "60"),
        error_answer(429, "rate_limit_error").with_header("retry-after", "60"),
        good_answer(),
    ])
    .await;
    let (provider, pacing) = provider_for(&stub);
    assert!(provider.complete(request()).await.is_ok());
    assert_eq!(pacing.pauses(), vec![sec(60), sec(60)]);
}

#[tokio::test]
async fn a_long_rate_limit_goes_back_to_the_queue_with_its_time() {
    let stub = Stub::start(vec![
        error_answer(429, "rate_limit_error").with_header("retry-after", "90"),
        good_answer(),
    ])
    .await;
    let (provider, pacing) = provider_for(&stub);
    let error = provider.complete(request()).await.unwrap_err();
    assert_eq!(error.retry_after(), Some(sec(90)));
    assert_eq!(stub.requests().len(), 1);
    assert!(pacing.pauses().is_empty());
}

#[tokio::test]
async fn a_rate_limit_that_names_no_time_is_not_guessed_at() {
    let stub = Stub::start(vec![error_answer(429, "rate_limit_error"), good_answer()]).await;
    let (provider, pacing) = provider_for(&stub);
    let error = provider.complete(request()).await.unwrap_err();
    assert!(matches!(error, ProviderError::Unavailable { retry_after: None }));
    assert_eq!(stub.requests().len(), 1);
    assert!(pacing.pauses().is_empty());
}

#[tokio::test]
async fn nobody_listening_is_retried_then_unavailable() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let pacing = Arc::new(TestPacing::default());
    let provider = AnthropicProvider::new(
        &AnthropicConfig::new(&url).unwrap(),
        &ApiKey::parse(TEST_KEY).unwrap(),
    )
    .unwrap()
    .with_pacing(pacing.clone());
    let error = provider.complete(request()).await.unwrap_err();
    assert!(
        matches!(error, ProviderError::Unavailable { retry_after: None }),
        "{error:?}"
    );
    assert_eq!(pacing.pauses().len(), 2);
}

// ---------- failures that are never retried ----------

#[tokio::test]
async fn failures_that_retrying_cannot_fix_are_tried_once() {
    let spend_cap = Answer::json(
        429,
        &json!({ "type": "error", "error": {
            "type": "rate_limit_error", "message": SECRET,
            "details": { "error_code": "enforced_spend_limit_reached" } } })
        .to_string(),
    );
    type Expect = fn(&ProviderError) -> bool;
    let cases: Vec<(Answer, Expect)> = vec![
        (error_answer(401, "authentication_error"), |e| {
            matches!(
                e,
                ProviderError::KeyProblem {
                    problem: KeyProblem::Rejected,
                    ..
                }
            )
        }),
        (error_answer(402, "billing_error"), |e| {
            matches!(
                e,
                ProviderError::KeyProblem {
                    problem: KeyProblem::Billing,
                    ..
                }
            )
        }),
        (spend_cap, |e| {
            matches!(
                e,
                ProviderError::KeyProblem {
                    problem: KeyProblem::SpendCap,
                    retry_after: Some(_)
                }
            )
        }),
        (error_answer(404, "not_found_error"), |e| {
            matches!(e, ProviderError::ModelUnavailable)
        }),
        (error_answer(400, "invalid_request_error"), |e| {
            matches!(e, ProviderError::Rejected(_))
        }),
        (error_answer(413, "request_too_large"), |e| {
            matches!(e, ProviderError::Rejected(_))
        }),
    ];
    for (answer, expected) in cases {
        let stub = Stub::start(vec![answer, good_answer()]).await;
        let (provider, pacing) = provider_for(&stub);
        let error = provider.complete(request()).await.unwrap_err();
        assert!(expected(&error), "{error:?}");
        assert_eq!(stub.requests().len(), 1, "{error:?}");
        assert!(pacing.pauses().is_empty());
        let shown = format!("{error:?} {error}");
        assert!(
            !shown.contains("SECRET"),
            "Anthropic's message must not be copied: {shown}"
        );
    }
}

#[tokio::test]
async fn the_monthly_cap_says_when_it_resets() {
    let stub = Stub::start(vec![Answer::json(
        429,
        &json!({ "type": "error", "error": { "type": "rate_limit_error", "message": "x",
            "details": { "error_code": "enforced_spend_limit_reached" } } })
        .to_string(),
    )])
    .await;
    let (provider, _) = provider_for(&stub);
    let error = provider.complete(request()).await.unwrap_err();
    // The test clock is 22 Oct 2026 12:00 UTC; the cap resets on 1 Nov 00:00 UTC.
    assert_eq!(error.retry_after(), Some(sec(1_793_491_200 - NOW)));
}

// ---------- time ----------

#[test]
fn the_time_allowed_grows_with_the_size_of_the_answer() {
    let policy = TimeoutPolicy::default();
    assert_eq!(policy.for_tokens(0), sec(30));
    assert_eq!(policy.for_tokens(2_000), sec(30 + 50));
    assert_eq!(policy.for_tokens(16_000), sec(30 + 400));
    assert_eq!(policy.for_tokens(u32::MAX), sec(30 + u64::from(u32::MAX) / 40));
}

#[tokio::test]
async fn a_server_that_is_too_slow_is_a_timeout_and_is_not_retried() {
    let stub = Stub::start(vec![good_answer().after(Duration::from_millis(600))]).await;
    let (provider, pacing) = provider_for(&stub);
    let provider = provider.with_timeouts(TimeoutPolicy {
        base: Duration::from_millis(100),
        tokens_per_second: 1_000_000,
    });
    let error = provider.complete(request()).await.unwrap_err();
    assert!(matches!(error, ProviderError::Timeout), "{error:?}");
    assert_eq!(error.failure_kind(), FailureKind::Transient);
    assert_eq!(stub.requests().len(), 1);
    assert!(pacing.pauses().is_empty());
}

// ---------- the answer is untrusted ----------

#[tokio::test]
async fn a_bad_answer_is_billed_and_not_retried_here() {
    let cut = Answer::json(
        200,
        &json!({ "model": "claude-haiku-5-5", "content": [{ "type": "text", "text": "{\"a\": \"hal" }],
                 "stop_reason": "max_tokens", "usage": { "input_tokens": 10, "output_tokens": 2000 } })
        .to_string(),
    );
    let stub = Stub::start(vec![cut, good_answer()]).await;
    let (provider, _) = provider_for(&stub);
    let error = provider.complete(request()).await.unwrap_err();
    assert!(matches!(&error, ProviderError::Truncated { usage } if usage.output_tokens == 2000));
    assert_eq!(stub.requests().len(), 1);
}

#[tokio::test]
async fn an_oversized_answer_is_not_read() {
    assert_eq!(MAX_RESPONSE_BYTES, 2 * 1024 * 1024);
    let big = format!("{{\"padding\":\"{}\"}}", "x".repeat(2 * 1024 * 1024 + 10));
    let stub = Stub::start(vec![Answer::json(200, &big)]).await;
    let (provider, _) = provider_for(&stub);
    let error = provider.complete(request()).await.unwrap_err();
    assert!(matches!(error, ProviderError::InvalidOutput { .. }), "{error:?}");
}

// ---------- the same contract as the fake ----------

/// What the pipeline relies on from any provider, whoever is behind it.
async fn contract<P: ModelProvider>(provider: &P) {
    let response: ModelResponse = provider.complete(request()).await.expect("a good answer");
    assert!(response.output.is_object(), "the output is a JSON object");
    assert!(!response.model.is_empty());
    assert!(
        format!("{response:?}").find("INV-1").is_none(),
        "Debug shows sizes only"
    );
}

#[tokio::test]
async fn the_fake_and_the_real_adapter_keep_the_same_contract() {
    let fake = FakeProvider::scripted(vec![Ok(ModelResponse {
        output: json!({ "invoice_number": "INV-1" }),
        usage: Usage::default(),
        model: "claude-haiku-5-5".into(),
        request_id: None,
    })]);
    contract(&fake).await;

    let stub = Stub::start(vec![good_answer()]).await;
    let (real, _) = provider_for(&stub);
    contract(&real).await;
}

#[tokio::test]
async fn the_time_allowed_comes_from_the_size_of_the_answer_asked_for() {
    // The request asks for 2000 tokens; at 10 tokens a second that is 200 s on top of 100 ms,
    // so a stub that takes 400 ms is fine. (Were the size ignored, 100 ms would not be enough.)
    let stub = Stub::start(vec![good_answer().after(Duration::from_millis(400))]).await;
    let (provider, _) = provider_for(&stub);
    let provider = provider.with_timeouts(TimeoutPolicy {
        base: Duration::from_millis(100),
        tokens_per_second: 10,
    });
    assert!(provider.complete(request()).await.is_ok());
}
