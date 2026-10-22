// Written by Kone Tshivhinda & Claude
use super::*;
use clearcut_core::jobs::FailureKind;
use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

// ---------- builders ----------

fn call() -> CallContext {
    CallContext {
        tenant: Uuid::from_u128(1),
        job: Some(Uuid::from_u128(2)),
        document: Some(Uuid::from_u128(3)),
        page: Some(4),
        purpose: Purpose::Extraction,
    }
}

fn schema() -> serde_json::Value {
    json!({ "type": "object", "properties": { "total": { "type": "integer" } }, "required": ["total"] })
}

/// The smallest valid request.
fn minimal() -> RequestBuilder {
    ModelRequest::builder("claude-haiku-5-5", call())
        .content(Block::text("Invoice INV-1, total R100.00"))
        .output_schema(schema())
        .max_output_tokens(2000)
}

fn png(len: usize) -> Block {
    Block::image(ImageType::Png, vec![7u8; len])
}

const MIB: usize = 1024 * 1024;

// ---------- a valid request ----------

#[test]
fn the_smallest_valid_request_builds_with_the_agreed_defaults() {
    let request = minimal().build().unwrap();
    assert_eq!(request.model(), "claude-haiku-5-5");
    assert_eq!(request.effort(), Effort::Medium);
    assert_eq!(request.thinking(), Thinking::Adaptive);
    assert_eq!(request.max_output_tokens(), 2000);
    assert_eq!(request.output_schema(), &schema());
    assert!(request.system().is_empty());
    assert_eq!(request.content().len(), 1);
    assert_eq!(request.call(), &call());
}

#[test]
fn everything_set_on_the_builder_comes_back_out() {
    let request = minimal()
        .system("You read South African invoices.")
        .cached_system("VAT rules ...")
        .content(png(10))
        .effort(Effort::High)
        .thinking(Thinking::Off)
        .build()
        .unwrap();
    assert_eq!(request.effort(), Effort::High);
    assert_eq!(request.thinking(), Thinking::Off);
    assert_eq!(
        request.system(),
        [
            TextBlock::plain("You read South African invoices."),
            TextBlock::cached("VAT rules ...")
        ]
    );
    assert_eq!(request.content().len(), 2);
    assert!(matches!(
        request.content()[1],
        Block::Image {
            media: ImageType::Png,
            ..
        }
    ));
}

#[test]
fn the_effort_and_thinking_names_are_the_apis() {
    assert_eq!(Effort::Low.as_str(), "low");
    assert_eq!(Effort::Medium.as_str(), "medium");
    assert_eq!(Effort::High.as_str(), "high");
    assert_eq!(ImageType::Png.media_type(), "image/png");
    assert_eq!(ImageType::Jpeg.media_type(), "image/jpeg");
    assert_eq!(ImageType::parse("image/png"), Some(ImageType::Png));
    assert_eq!(ImageType::parse("image/jpeg"), Some(ImageType::Jpeg));
    for bad in ["image/gif", "IMAGE/PNG", " image/png", "application/pdf", ""] {
        assert_eq!(ImageType::parse(bad), None, "{bad:?}");
    }
}

// ---------- what a request must have ----------

#[test]
fn a_model_id_is_a_plain_identifier() {
    for good in [
        "claude-haiku-5-5",
        "claude-fable-5-1",
        "us.anthropic.claude-opus-5-5-v1:0",
        "m",
    ] {
        assert!(
            ModelRequest::builder(good, call())
                .content(Block::text("x"))
                .output_schema(schema())
                .max_output_tokens(10)
                .build()
                .is_ok(),
            "{good}"
        );
    }
    for bad in [
        "",
        " ",
        "claude haiku",
        "claude\n",
        "a/b",
        "claude-haiku-5-5 ",
        "modèle",
    ] {
        let result = ModelRequest::builder(bad, call())
            .content(Block::text("x"))
            .output_schema(schema())
            .max_output_tokens(10)
            .build();
        assert_eq!(result.unwrap_err(), RequestError::BadModel, "{bad:?}");
    }
}

#[test]
fn a_model_id_has_a_length_limit() {
    let with_id = |id: String| {
        ModelRequest::builder(id, call())
            .content(Block::text("x"))
            .output_schema(schema())
            .max_output_tokens(10)
            .build()
    };
    assert!(with_id("m".repeat(128)).is_ok());
    assert_eq!(with_id("m".repeat(129)).unwrap_err(), RequestError::BadModel);
}

#[test]
fn a_request_needs_something_to_read() {
    let result = ModelRequest::builder("m", call())
        .output_schema(schema())
        .max_output_tokens(10)
        .build();
    assert_eq!(result.unwrap_err(), RequestError::NoContent);
}

#[test]
fn blank_text_is_refused_wherever_it_appears() {
    for blank in ["", " ", "\n\t "] {
        assert_eq!(
            minimal().content(Block::text(blank)).build().unwrap_err(),
            RequestError::BlankText,
            "content {blank:?}"
        );
        assert_eq!(
            minimal().system(blank).build().unwrap_err(),
            RequestError::BlankText,
            "system {blank:?}"
        );
        assert_eq!(
            minimal().cached_system(blank).build().unwrap_err(),
            RequestError::BlankText,
            "cached system {blank:?}"
        );
    }
}

#[test]
fn the_output_schema_is_required_and_must_be_an_object() {
    let missing = ModelRequest::builder("m", call())
        .content(Block::text("x"))
        .max_output_tokens(10)
        .build();
    assert_eq!(missing.unwrap_err(), RequestError::NoSchema);
    for bad in [json!([]), json!("object"), json!(null), json!(3), json!(true)] {
        assert_eq!(
            minimal().output_schema(bad.clone()).build().unwrap_err(),
            RequestError::SchemaNotAnObject,
            "{bad}"
        );
    }
    assert!(minimal().output_schema(json!({})).build().is_ok());
}

#[test]
fn the_output_token_cap_is_required_and_within_range() {
    assert_eq!(MAX_OUTPUT_TOKENS, 64_000);
    let missing = ModelRequest::builder("m", call())
        .content(Block::text("x"))
        .output_schema(schema())
        .build();
    assert_eq!(missing.unwrap_err(), RequestError::BadTokenCap);
    for bad in [0, MAX_OUTPUT_TOKENS + 1, u32::MAX] {
        assert_eq!(
            minimal().max_output_tokens(bad).build().unwrap_err(),
            RequestError::BadTokenCap,
            "{bad}"
        );
    }
    for good in [1, MAX_OUTPUT_TOKENS] {
        assert!(minimal().max_output_tokens(good).build().is_ok(), "{good}");
    }
}

// ---------- the API's own limits ----------

#[test]
fn an_image_must_have_bytes_and_be_no_bigger_than_the_api_allows() {
    assert_eq!(MAX_IMAGE_BYTES, 5 * MIB);
    assert_eq!(
        minimal().content(png(0)).build().unwrap_err(),
        RequestError::EmptyImage
    );
    assert_eq!(
        minimal().content(png(MAX_IMAGE_BYTES + 1)).build().unwrap_err(),
        RequestError::ImageTooLarge
    );
    assert!(minimal().content(png(MAX_IMAGE_BYTES)).build().is_ok());
}

#[test]
fn there_is_a_limit_on_how_many_images_one_request_carries() {
    assert_eq!(MAX_IMAGES, 20);
    let mut builder = minimal();
    for _ in 0..MAX_IMAGES {
        builder = builder.content(png(10));
    }
    assert!(builder.clone().build().is_ok());
    assert_eq!(
        builder.content(png(10)).build().unwrap_err(),
        RequestError::TooManyImages
    );
}

#[test]
fn at_most_four_cache_breakpoints_across_the_whole_request() {
    assert_eq!(MAX_CACHE_BREAKPOINTS, 4);
    let four = minimal()
        .cached_system("a")
        .cached_system("b")
        .content(Block::cached_text("c"))
        .content(Block::cached_text("d"));
    assert!(four.clone().build().is_ok());
    assert_eq!(
        four.content(Block::cached_text("e")).build().unwrap_err(),
        RequestError::TooManyCacheBreakpoints
    );
    // Plain text is never a breakpoint, however much of it there is.
    let mut many = minimal();
    for n in 0..30 {
        many = many.content(Block::text(format!("page {n}")));
    }
    assert!(many.build().is_ok());
}

#[test]
fn the_whole_request_has_a_size_cap() {
    assert_eq!(MAX_REQUEST_BYTES, 25 * MIB);
    let five_images = || {
        let mut b = ModelRequest::builder("m", call())
            .output_schema(schema())
            .max_output_tokens(10);
        for _ in 0..5 {
            b = b.content(png(MAX_IMAGE_BYTES));
        }
        b
    };
    assert!(five_images().build().is_ok(), "exactly at the cap");
    assert_eq!(
        five_images().content(Block::text("x")).build().unwrap_err(),
        RequestError::TooLarge,
        "one byte over"
    );
    // "é" is one character but two bytes: half the cap in characters is already over it in bytes.
    assert_eq!(
        minimal()
            .content(Block::text("é".repeat(MAX_REQUEST_BYTES / 2)))
            .build()
            .unwrap_err(),
        RequestError::TooLarge,
        "text counts in bytes, not characters"
    );
    assert_eq!(
        minimal()
            .system("s".repeat(MAX_REQUEST_BYTES))
            .build()
            .unwrap_err(),
        RequestError::TooLarge,
        "instructions count too"
    );
}

// ---------- invoice content never reaches a log ----------

#[test]
fn a_request_prints_its_sizes_and_never_its_content() {
    let request = minimal()
        .system("SECRET-SYSTEM-TEXT")
        .content(Block::text("SECRET-PAGE-TEXT bank 1234567890"))
        .content(Block::image(ImageType::Png, b"SECRETPIXELS".to_vec()))
        .build()
        .unwrap();
    for shown in [format!("{request:?}"), format!("{request:#?}")] {
        for secret in ["SECRET", "1234567890", "INV-1", "R100.00", "5345", "SECRETPIXELS"] {
            assert!(!shown.contains(secret), "{secret} leaked into {shown}");
        }
        assert!(shown.contains("claude-haiku-5-5"));
        assert!(shown.contains("bytes"), "sizes are shown: {shown}");
    }
    assert!(!format!("{:?}", Block::text("SECRET")).contains("SECRET"));
    assert!(!format!("{:?}", TextBlock::cached("SECRET")).contains("SECRET"));
    assert!(!format!("{:?}", Block::image(ImageType::Png, b"SECRET".to_vec())).contains("SECRET"));
    assert_eq!(
        format!("{:?}", Block::image(ImageType::Png, b"SECRET".to_vec())),
        "Image(image/png, 6 bytes)"
    );
    assert_eq!(
        format!("{:?}", TextBlock::cached("SECRET")),
        "TextBlock(6 bytes, cache: true)"
    );
    let sizes = format!("{request:?}");
    // "SECRET-SYSTEM-TEXT" is 18 bytes; the content is 28 + 32 text bytes and 12 image bytes.
    assert!(sizes.contains("1 blocks, 18 bytes"), "{sizes}");
    assert!(sizes.contains("3 blocks (1 images), 72 bytes"), "{sizes}");
}

#[test]
fn a_response_prints_its_usage_and_never_the_extracted_data() {
    let response = ModelResponse {
        output: json!({ "supplier": "SECRET-SUPPLIER", "total": 123456 }),
        usage: Usage {
            input_tokens: 1000,
            output_tokens: 50,
            ..Usage::default()
        },
        model: "claude-haiku-5-5".into(),
        request_id: Some("req_1".into()),
    };
    let shown = format!("{response:?}");
    assert!(!shown.contains("SECRET") && !shown.contains("123456"), "{shown}");
    assert!(shown.contains("1000") && shown.contains("claude-haiku-5-5") && shown.contains("req_1"));
}

// ---------- usage ----------

#[test]
fn usage_adds_up_and_cannot_overflow() {
    let a = Usage {
        input_tokens: 1000,
        output_tokens: 200,
        cache_read_tokens: 30,
        cache_write_tokens: 4,
    };
    let b = Usage {
        input_tokens: 1,
        output_tokens: 2,
        cache_read_tokens: 3,
        cache_write_tokens: 4,
    };
    let sum = a + b;
    assert_eq!(
        sum,
        Usage {
            input_tokens: 1001,
            output_tokens: 202,
            cache_read_tokens: 33,
            cache_write_tokens: 8
        }
    );
    assert_eq!(sum.total(), 1001 + 202 + 33 + 8);
    assert_eq!(Usage::default().total(), 0);
    let big = Usage {
        input_tokens: u64::MAX,
        output_tokens: u64::MAX,
        cache_read_tokens: u64::MAX,
        cache_write_tokens: u64::MAX,
    };
    assert_eq!((big + big).input_tokens, u64::MAX, "saturates");
    assert_eq!(big.total(), u64::MAX, "saturates");
    for position in 0..4 {
        // Two huge counts and two zeros, wherever the huge ones sit: still no wrap-around.
        for other in (0..4).filter(|&p| p != position) {
            let mut counts = [0u64; 4];
            counts[position] = u64::MAX;
            counts[other] = u64::MAX;
            let usage = Usage {
                input_tokens: counts[0],
                output_tokens: counts[1],
                cache_read_tokens: counts[2],
                cache_write_tokens: counts[3],
            };
            assert_eq!(usage.total(), u64::MAX, "positions {position} and {other}");
        }
    }
    let mut running = Usage::default();
    running += a;
    running += b;
    assert_eq!(running, sum);
    assert_eq!([a, b, a].into_iter().sum::<Usage>(), a + b + a);
}

// ---------- errors ----------

#[test]
fn each_provider_error_maps_to_the_queues_failure_kind() {
    let usage = Usage {
        input_tokens: 10,
        ..Usage::default()
    };
    let table: Vec<(ProviderError, FailureKind)> = vec![
        (
            ProviderError::Unavailable { retry_after: None },
            FailureKind::ProviderUnavailable,
        ),
        (
            ProviderError::Unavailable {
                retry_after: Some(Duration::from_secs(30)),
            },
            FailureKind::ProviderUnavailable,
        ),
        (ProviderError::Auth, FailureKind::ProviderUnavailable),
        (ProviderError::Timeout, FailureKind::Transient),
        (ProviderError::Truncated { usage }, FailureKind::Transient),
        (
            ProviderError::InvalidOutput {
                usage,
                reason: "missing field total".into(),
            },
            FailureKind::Transient,
        ),
        (ProviderError::Refused { usage }, FailureKind::NeedsReview),
        (
            ProviderError::Rejected("bad request".into()),
            FailureKind::NeedsReview,
        ),
    ];
    for (error, kind) in table {
        assert_eq!(error.failure_kind(), kind, "{error}");
    }
}

#[test]
fn errors_that_were_still_billed_carry_their_usage() {
    let usage = Usage {
        input_tokens: 5000,
        output_tokens: 1000,
        ..Usage::default()
    };
    for billed in [
        ProviderError::Truncated { usage },
        ProviderError::Refused { usage },
        ProviderError::InvalidOutput {
            usage,
            reason: "x".into(),
        },
    ] {
        assert_eq!(billed.usage(), Some(usage), "{billed}");
    }
    for free in [
        ProviderError::Unavailable { retry_after: None },
        ProviderError::Timeout,
        ProviderError::Auth,
        ProviderError::Rejected("x".into()),
    ] {
        assert_eq!(free.usage(), None, "{free}");
    }
}

#[test]
fn only_an_unavailable_provider_can_say_when_to_come_back() {
    let wait = Duration::from_secs(42);
    assert_eq!(
        ProviderError::Unavailable {
            retry_after: Some(wait)
        }
        .retry_after(),
        Some(wait)
    );
    assert_eq!(
        ProviderError::Unavailable { retry_after: None }.retry_after(),
        None
    );
    assert_eq!(ProviderError::Timeout.retry_after(), None);
}

#[test]
fn error_messages_are_plain_and_carry_only_the_stated_reason() {
    let invalid = ProviderError::InvalidOutput {
        usage: Usage::default(),
        reason: "missing field total".into(),
    };
    assert_eq!(
        invalid.to_string(),
        "the model's output was not valid: missing field total"
    );
    assert_eq!(ProviderError::Timeout.to_string(), "the model provider timed out");
    assert_eq!(
        ProviderError::Auth.to_string(),
        "the model provider refused our credentials"
    );
    assert_eq!(
        ProviderError::Rejected("max_tokens too large".into()).to_string(),
        "the model provider rejected the request: max_tokens too large"
    );
    // It is a real error type, usable with `?` and `dyn Error`.
    let boxed: Box<dyn std::error::Error> = Box::new(ProviderError::Timeout);
    assert!(!boxed.to_string().is_empty());
}

// ---------- the scripted fake ----------

fn answer(total: i64) -> ModelResponse {
    ModelResponse {
        output: json!({ "total": total }),
        usage: Usage {
            input_tokens: 100,
            output_tokens: 10,
            ..Usage::default()
        },
        model: "claude-haiku-5-5".into(),
        request_id: None,
    }
}

/// Generic over the trait, as the pipeline will be.
async fn ask<P: ModelProvider>(provider: &P) -> Result<ModelResponse, ProviderError> {
    provider.complete(minimal().build().unwrap()).await
}

#[tokio::test]
async fn the_fake_gives_its_scripted_answers_in_order_and_then_says_it_has_none() {
    let fake = FakeProvider::scripted(vec![Ok(answer(1)), Err(ProviderError::Timeout), Ok(answer(3))]);
    assert_eq!(ask(&fake).await.unwrap().output, json!({ "total": 1 }));
    assert!(matches!(ask(&fake).await, Err(ProviderError::Timeout)));
    assert_eq!(ask(&fake).await.unwrap().output, json!({ "total": 3 }));
    match ask(&fake).await {
        Err(ProviderError::Rejected(reason)) => {
            assert!(reason.contains("no more scripted answers"), "{reason}")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn the_fake_remembers_every_request_it_was_asked() {
    let fake = FakeProvider::scripted(vec![Ok(answer(1)), Ok(answer(2))]);
    assert_eq!(fake.call_count(), 0);
    let first = minimal().build().unwrap();
    let second = minimal().content(Block::text("another page")).build().unwrap();
    fake.complete(first.clone()).await.unwrap();
    fake.complete(second.clone()).await.unwrap();
    assert_eq!(fake.call_count(), 2);
    assert_eq!(fake.requests(), [first, second]);
}

#[tokio::test]
async fn the_fake_counts_a_call_even_when_it_has_nothing_to_say() {
    let fake = FakeProvider::scripted(vec![]);
    let _ = ask(&fake).await;
    assert_eq!(fake.call_count(), 1);
}

#[tokio::test]
async fn the_fake_can_answer_from_the_request() {
    let fake = FakeProvider::answering(|request| {
        // Answers with the number of content blocks, to show it saw the request.
        Ok(answer(request.content().len() as i64))
    });
    let one = fake.complete(minimal().build().unwrap()).await.unwrap();
    let three = fake
        .complete(
            minimal()
                .content(Block::text("b"))
                .content(Block::text("c"))
                .build()
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(one.output, json!({ "total": 1 }));
    assert_eq!(three.output, json!({ "total": 3 }));
    assert_eq!(fake.call_count(), 2);
}

#[tokio::test]
async fn clones_of_the_fake_share_one_script_and_one_record() {
    let fake = FakeProvider::scripted(vec![Ok(answer(1)), Ok(answer(2))]);
    let other = fake.clone();
    ask(&fake).await.unwrap();
    assert_eq!(ask(&other).await.unwrap().output, json!({ "total": 2 }));
    assert_eq!(fake.call_count(), 2);
    assert_eq!(other.call_count(), 2);
}
