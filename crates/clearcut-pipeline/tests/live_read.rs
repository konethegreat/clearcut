// Written by Kone Tshivhinda & Claude
//! One real page through the whole path: the real worker, the real Anthropic API, a real
//! Postgres (ADR-0018 step 8).
//!
//! **Ignored by default, and never run in CI** (CI has no key). It costs a little: one page on
//! Haiku 5.5 is about R0.02, and the document's budget is capped at R5 here (the default),
//! so even a page that climbed every rung could not cost more than that. It exists so that the first time the
//! whole path meets the real API is on purpose, with your own key, and not in the middle of a
//! real invoice. It needs:
//!
//! ```text
//! ANTHROPIC_API_KEY=sk-ant-api03-...        your own key (a dedicated workspace key with a low spend limit)
//! CLEARCUT_LIVE_WORKER_URL=http://127.0.0.1:8090    the running worker (with OCR for the best result)
//! CLEARCUT_LIVE_WORKER_TOKEN=<the worker's token>
//! DATABASE_URL=postgres://clearcut@127.0.0.1:5432/clearcut
//!
//! cargo test -p clearcut-pipeline --test live_read -- --ignored --nocapture
//! ```
//!
//! It prints what happened: the rung that read the page, every call with its tokens and what it
//! was charged in rand, and the month's spending. **The first run should be looked at by a
//! person**: it is the first measurement of the pass rate, the cost and the cache that
//! ADR-0018 only guesses. The text of the invoice is invented; nothing here is a real document.

use clearcut_anthropic::provider::AnthropicProvider;
use clearcut_anthropic::{load, AnthropicConfig};
use clearcut_core::cost::{BudgetPolicy, Cost};
use clearcut_core::jobs::{JobKind, JobState};
use clearcut_docproc::HttpWorker;
use clearcut_jobs::{Ended, Runner, RunnerConfig};
use clearcut_objects::{LocalDisk, ObjectStore, PutLimits};
use clearcut_pipeline::{PipelineConfig, PipelineHandler, ReadSettings};
use clearcut_store::page_readings::PageState;
use clearcut_store::{NewDocument, NewJob, Store};
use sqlx::PgPool;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

const INVOICE: [&str; 9] = [
    "TAX INVOICE",
    "Invoice Number INV-2024-001",
    "Invoice Date 2024-03-01",
    "Description Quantity Unit Price Amount",
    "Consulting services March 10 150.00 1500.00",
    "Subtotal 1500.00",
    "VAT at 15 percent 225.00",
    "Total amount due R1725.00",
    "Thank you for your business",
];

/// A one-page PDF in Helvetica that says exactly what `INVOICE` says (invented text).
fn pdf() -> Vec<u8> {
    let mut stream = String::from("BT\n/F1 14 Tf\n24 TL\n50 780 Td\n");
    for line in INVOICE {
        let _ = writeln!(stream, "({line}) Tj T*");
    }
    stream.push_str("ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
         /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R >>"
            .to_string(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len() + 1),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        let _ = write!(out, "{} 0 obj\n{body}\nendobj\n", index + 1);
    }
    let xref = out.len();
    let _ = write!(out, "xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for offset in offsets {
        let _ = writeln!(out, "{offset:010} 00000 n ");
    }
    let _ = write!(
        out,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    );
    out.into_bytes()
}

/// Nano-rand as "R0.019800": whole numbers only, no floats for money.
fn rand(cost: Cost) -> String {
    let nano = cost.nano_rand();
    format!("R{}.{:06}", nano / 1_000_000_000, (nano % 1_000_000_000) / 1_000)
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
#[ignore = "needs your own ANTHROPIC_API_KEY, the real worker and a database; costs about R0.02, at most R5"]
async fn one_real_page_is_read_by_the_real_model_and_the_cost_is_logged(pool: PgPool) {
    let key = load().expect("set ANTHROPIC_API_KEY (or ANTHROPIC_API_KEY_FILE)");
    let anthropic = AnthropicConfig::from_env(|name| std::env::var_os(name)).expect("a valid configuration");
    let provider = AnthropicProvider::new(&anthropic, &key).expect("a client");
    let worker = HttpWorker::new(
        &std::env::var("CLEARCUT_LIVE_WORKER_URL").expect("set CLEARCUT_LIVE_WORKER_URL"),
        &std::env::var("CLEARCUT_LIVE_WORKER_TOKEN").expect("set CLEARCUT_LIVE_WORKER_TOKEN"),
    )
    .expect("a worker client");

    // The restricted role the service runs as, a real object store on disk.
    let tenant = Store::new(pool.clone())
        .create_tenant("Live Advisory")
        .await
        .unwrap();
    let options = pool.connect_options().as_ref().clone().username("clearcut_app");
    let app_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    let store = Store::new(app_pool);
    let dir = tempfile::tempdir().unwrap();
    let objects = LocalDisk::new(dir.path());
    let bytes = pdf();
    let stored = objects
        .put(tenant.0, &bytes[..], PutLimits { max_bytes: 2_000_000 })
        .await
        .unwrap();
    let doc = store
        .add_document(
            tenant,
            &NewDocument {
                original_filename: "invoice.pdf".into(),
                content_type: "application/pdf".into(),
                byte_size: i64::try_from(stored.byte_size).unwrap(),
                sha256: *stored.sha256.as_bytes(),
                storage_key: stored.key.as_str().to_string(),
                page_count: None,
            },
        )
        .await
        .unwrap()
        .id;
    let job = store
        .enqueue_job(
            tenant,
            &NewJob::new(doc, JobKind::ProcessDocument),
            &serde_json::json!({ "type": "system" }),
        )
        .await
        .unwrap()
        .id;

    // A cap of R5 for the document (the default; a cap of R1 would not let a page climb to Sonnet,
    // whose worst case is about R2.70) and R20 for the month.
    let budget = BudgetPolicy::new(
        Cost::from_rand_cents(500),
        Cost::from_rand_cents(500),
        Cost::from_rand_cents(2_000),
    )
    .unwrap();
    let handler = PipelineHandler::new(
        store.clone(),
        objects,
        Arc::new(worker),
        PipelineConfig::default(),
    )
    .with_models(Arc::new(provider), ReadSettings::standard().with_budget(budget));
    let config = RunnerConfig::new("live-read")
        .unwrap()
        .with_heartbeat_every(Duration::from_millis(500))
        .unwrap();
    let report = Runner::new(store.clone(), handler, config)
        .run_once()
        .await
        .unwrap()
        .expect("a job was due");

    // What happened, for a person to look at.
    println!("job ended: {:?}", report.ended);
    let pages = store.load_page_readings(tenant, doc).await.unwrap();
    for page in &pages {
        match &page.state {
            PageState::Read(kept) => println!(
                "page {}: read by rung {} ({}), attempt {}, evidence {:?}",
                page.page, kept.rung, kept.model, kept.attempt, kept.confidence
            ),
            other => println!("page {}: {other:?}", page.page),
        }
        println!("  problems: {}", page.details.problems);
        println!(
            "  not confirmed by the page's own text: {:?}",
            page.details.unconfirmed
        );
    }
    type CallRow = (i32, String, String, Option<String>, i64, i64, i64, i64, i64);
    let calls: Vec<CallRow> = sqlx::query_as(
        "select attempt, model, label, label_detail, input_tokens, output_tokens, cache_read_tokens, \
         cache_write_tokens, cost_nano_rand from model_calls where document_id = $1 order by id",
    )
    .bind(doc.0)
    .fetch_all(&pool)
    .await
    .unwrap();
    for (attempt, model, label, detail, input, output, cache_read, cache_write, nano) in &calls {
        println!(
            "call {attempt}: {model} {label} {detail:?}; tokens in {input}, out {output}, \
             cache read {cache_read}, cache write {cache_write}; {}",
            rand(Cost::from_nano_rand(u64::try_from(*nano).unwrap_or(0)))
        );
    }
    let spend = store.document_spend(tenant, doc).await.unwrap();
    println!("the document cost {} in {} calls", rand(spend.cost), spend.calls);

    // The path worked, within the money it was allowed.
    assert_eq!(report.ended, Ended::Succeeded, "{report:?}");
    let job = store.get_job(tenant, job).await.unwrap();
    assert_eq!(job.state, JobState::Succeeded, "{:?}", job.last_error);
    let events = store.load_events(tenant, doc).await.unwrap();
    println!("{} events were stored for the document", events.len());
    assert!(!events.is_empty(), "the document was turned into events");
    assert_eq!(pages.len(), 1);
    let PageState::Read(kept) = &pages[0].state else {
        panic!("the page was not read: {:?}", pages[0].state);
    };
    assert_eq!(kept.reading["invoice_number"], "INV-2024-001");
    assert_eq!(kept.reading["totals"]["gross"], "1725.00");
    assert!(!calls.is_empty(), "the call is in the log");
    assert!(spend.cost > Cost::ZERO, "a real call costs something");
    assert!(
        spend.cost <= Cost::from_rand_cents(500),
        "and no more than the cap"
    );
}

/// The one part of this file that needs no key: the PDF it builds is well formed, with every
/// cross-reference offset pointing at the object it names, and says what `INVOICE` says.
#[test]
fn the_pdf_it_builds_is_well_formed() {
    let bytes = pdf();
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        text.starts_with(
            "%PDF-1.4
"
        ) && text.ends_with(
            "%%EOF
"
        )
    );
    let table = text
        .split(
            "xref
",
        )
        .nth(1)
        .expect("a cross-reference table");
    let offsets: Vec<usize> = table
        .lines()
        .skip(2)
        .take(5)
        .map(|line| line[..10].parse().unwrap())
        .collect();
    assert_eq!(offsets.len(), 5);
    for (index, offset) in offsets.into_iter().enumerate() {
        assert!(
            text[offset..].starts_with(&format!("{} 0 obj", index + 1)),
            "object {} is not at {offset}",
            index + 1
        );
    }
    let start: usize = text
        .rsplit(
            "startxref
",
        )
        .next()
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(text[start..].starts_with("xref"));
    for line in INVOICE {
        assert!(text.contains(&format!("({line}) Tj")), "{line}");
    }
}
