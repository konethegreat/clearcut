// Written by Kone Tshivhinda & Claude
//! The analysis stage against the **real** Python worker, with PDFs whose text layer lies
//! (ADR-0017 step 4, and the detection half of P2-20).
//!
//! The unit tests script the worker; this one proves the whole path works with the real
//! PDF reader and the real Tesseract. It is `#[ignore]`d so that `cargo test --workspace`
//! stays self-contained, and CI runs it in the `contract` job, after starting the worker
//! with OCR and a Postgres service:
//!
//! ```text
//! CLEARCUT_LIVE_WORKER_URL=http://127.0.0.1:8090 CLEARCUT_LIVE_WORKER_TOKEN=<the worker's token> \
//! DATABASE_URL=postgres://clearcut@127.0.0.1:5432/clearcut \
//!     cargo test -p clearcut-pipeline --test live -- --ignored
//! ```
//!
//! **Ground truth by construction.** Each PDF is written here, by a few lines of code, with
//! a standard font that draws exactly what the page says and a `ToUnicode` map that tells
//! a text extractor something else: every letter is the next letter of the alphabet (or
//! every digit the next digit). The picture is right, the text layer is wrong, and we know
//! exactly how. That is the case ADR-0016 says the text measures alone cannot catch (the
//! text has the right alphabet and, for the digits, is valid text), and which only the OCR
//! agreement settles. Every text here is invented.

use bytes::Bytes;
use clearcut_core::jobs::{JobKind, JobState};
use clearcut_core::page_read::{PageRead, ReadRoute, ROUTING_VERSION};
use clearcut_core::text_quality::{Reason, TextVerdict};
use clearcut_docproc::{DocumentProcessor, HttpWorker, PageRange};
use clearcut_jobs::{Ended, Runner, RunnerConfig};
use clearcut_objects::{LocalDisk, ObjectStore, PutLimits};
use clearcut_pipeline::{PipelineConfig, PipelineHandler, AFTER_ANALYSIS};
use clearcut_store::{NewDocument, NewJob, Store};
use sqlx::PgPool;
use std::fmt::Write as _;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

// ---------- PDFs whose text layer lies ----------

/// What the hidden map does to the characters a text extractor sees.
#[derive(Clone, Copy)]
struct Cipher {
    letters: bool,
    digits: bool,
}

const HONEST: Option<Cipher> = None;
const LETTERS_SHIFTED: Option<Cipher> = Some(Cipher {
    letters: true,
    digits: false,
});
const DIGITS_SHIFTED: Option<Cipher> = Some(Cipher {
    letters: false,
    digits: true,
});

impl Cipher {
    /// The character a text extractor is told for `c`: the next letter, the next digit.
    fn apply(self, c: char) -> char {
        match c {
            'a'..='z' if self.letters => (((c as u8 - b'a' + 1) % 26) + b'a') as char,
            'A'..='Z' if self.letters => (((c as u8 - b'A' + 1) % 26) + b'A') as char,
            '0'..='9' if self.digits => (((c as u8 - b'0' + 1) % 10) + b'0') as char,
            other => other,
        }
    }

    fn text(self, text: &str) -> String {
        text.chars().map(|c| self.apply(c)).collect()
    }

    /// The `ToUnicode` CMap: one entry for each character the cipher changes.
    fn cmap(self) -> String {
        let mut entries = String::new();
        let mut count = 0;
        for code in (b'0'..=b'9').chain(b'A'..=b'Z').chain(b'a'..=b'z') {
            let shown = self.apply(code as char);
            if shown as u32 != u32::from(code) {
                let _ = writeln!(entries, "<{code:02X}> <{:04X}>", shown as u32);
                count += 1;
            }
        }
        format!(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
             /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
             /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
             1 begincodespacerange\n<00> <FF>\nendcodespacerange\n\
             {count} beginbfchar\n{entries}endbfchar\nendcmap\n\
             CMapName currentdict /CMap defineresource pop\nend\nend"
        )
    }
}

const PAGE_ONE: [&str; 9] = [
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

const PAGE_TWO: [&str; 5] = [
    "Terms and conditions",
    "Payment is due within thirty days of the invoice date",
    "Please use the invoice number as the payment reference",
    "Late payments may attract interest on the outstanding balance",
    "Questions about this invoice should be sent to the accounts team",
];

/// A two-page PDF in Helvetica. With a cipher, the font carries a `ToUnicode` map that
/// disagrees with what it draws.
fn pdf(cipher: Option<Cipher>) -> Vec<u8> {
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [5 0 R 7 0 R] /Count 2 >>".to_string(),
    ];
    let font = if cipher.is_some() {
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /ToUnicode 4 0 R >>"
    } else {
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
    };
    objects.push(font.to_string());
    let cmap = cipher.map(Cipher::cmap).unwrap_or_default();
    objects.push(format!(
        "<< /Length {} >>\nstream\n{cmap}\nendstream",
        cmap.len() + 1
    ));
    for (content_object, lines) in [(6, &PAGE_ONE[..]), (8, &PAGE_TWO[..])] {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F1 3 0 R >> >> /Contents {content_object} 0 R >>"
        ));
        let mut stream = String::from("BT\n/F1 14 Tf\n24 TL\n50 780 Td\n");
        for line in lines {
            let _ = writeln!(stream, "({line}) Tj T*");
        }
        stream.push_str("ET");
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len() + 1
        ));
    }

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

// ---------- the real worker, the real store ----------

fn live_worker() -> HttpWorker {
    let url = std::env::var("CLEARCUT_LIVE_WORKER_URL")
        .expect("set CLEARCUT_LIVE_WORKER_URL to the running worker, for example http://127.0.0.1:8090");
    let token = std::env::var("CLEARCUT_LIVE_WORKER_TOKEN").expect("set CLEARCUT_LIVE_WORKER_TOKEN");
    HttpWorker::new(&url, &token).unwrap()
}

struct Analysed {
    reads: Vec<PageRead>,
    ocr_pages: u64,
    ocr: String,
}

/// Stores `bytes` as a document, runs the analysis with the real worker, and returns what it decided.
async fn analyse(pool: &PgPool, bytes: Vec<u8>) -> Analysed {
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

    let handler = PipelineHandler::new(
        store.clone(),
        objects,
        Arc::new(live_worker()),
        PipelineConfig::default(),
    );
    let config = RunnerConfig::new("live-test")
        .unwrap()
        .with_heartbeat_every(Duration::from_millis(500))
        .unwrap();
    let report = Runner::new(store.clone(), handler, config)
        .run_once()
        .await
        .unwrap()
        .expect("a job was due");
    assert_eq!(report.ended, Ended::NeedsReview, "{report:?}");
    let job = store.get_job(tenant, job).await.unwrap();
    assert_eq!(job.state, JobState::NeedsReview);
    assert_eq!(
        job.last_error.as_deref(),
        Some(AFTER_ANALYSIS),
        "the analysis itself succeeded"
    );

    let detail: serde_json::Value = sqlx::query_scalar(
        "select detail from audit_log where action = 'document.analysed' and subject_id = $1",
    )
    .bind(doc.0.to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    Analysed {
        reads: store.load_page_reads(tenant, doc).await.unwrap(),
        ocr_pages: detail["ocr_pages"].as_u64().unwrap(),
        ocr: detail["ocr"].as_str().unwrap().to_string(),
    }
}

/// The text the real PDF reader finds on page `page` of `bytes`.
async fn text_layer(bytes: &[u8], page: u32) -> String {
    let n = NonZeroU32::new(page).unwrap();
    let pages = live_worker()
        .page_texts(Bytes::copy_from_slice(bytes), PageRange::single(n), false)
        .await
        .unwrap();
    pages[0].text().to_string()
}

fn words_of(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------- the honest document, and the two lies ----------

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
#[ignore = "needs the real worker with OCR and a database; run by the CI contract job"]
async fn an_honest_pdf_is_trusted_after_one_ocr_of_its_first_page(pool: PgPool) {
    let bytes = pdf(HONEST);
    // The text layer says what the page says.
    assert!(words_of(&text_layer(&bytes, 1).await).contains("Invoice Number INV-2024-001"));

    let analysed = analyse(&pool, bytes).await;
    assert_eq!(analysed.ocr, "ran", "the worker must have OCR for this test");
    assert_eq!(analysed.ocr_pages, 1, "only the first page is the sentinel");
    assert_eq!(analysed.reads.len(), 2);
    for read in &analysed.reads {
        assert_eq!(read.verdict(), TextVerdict::Good, "{read:?}");
        assert_eq!(read.route(), ReadRoute::TextAndImage);
        assert!(!read.text_layer_rejected());
        assert_eq!(read.analysed_with(), ROUTING_VERSION);
    }
    let agreement = analysed.reads[0]
        .ocr_agreement()
        .expect("page one was compared with OCR");
    assert!(
        agreement >= 850,
        "an honest page agrees with OCR, got {agreement}"
    );
    assert_eq!(analysed.reads[0].reasons(), &[Reason::OcrAgrees]);
    assert_eq!(analysed.reads[1].ocr_agreement(), None);
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
#[ignore = "needs the real worker with OCR and a database; run by the CI contract job"]
async fn a_pdf_whose_letters_are_shifted_is_caught_and_read_from_its_pictures(pool: PgPool) {
    let bytes = pdf(LETTERS_SHIFTED);
    // Ground truth: the picture says "Invoice Number"; the text layer says the next letters.
    let found = words_of(&text_layer(&bytes, 1).await);
    let cipher = LETTERS_SHIFTED.unwrap();
    assert!(
        found.contains(&cipher.text("Invoice Number INV-2024-001")),
        "{found}"
    );
    assert!(!found.contains("Invoice"), "{found}");

    let analysed = analyse(&pool, bytes).await;
    assert_eq!(analysed.ocr, "ran");
    // Page one disagreed with OCR, so the rest of the document was read by OCR as well.
    assert_eq!(analysed.ocr_pages, 2);
    assert_eq!(analysed.reads.len(), 2);
    for read in &analysed.reads {
        assert_eq!(read.verdict(), TextVerdict::Unusable, "{read:?}");
        assert_eq!(read.route(), ReadRoute::ImageOnly);
        assert!(read.text_layer_rejected());
        assert_eq!(read.reasons(), &[Reason::OcrDisagrees]);
        let agreement = read.ocr_agreement().expect("compared with OCR");
        assert!(
            agreement < 500,
            "a shifted alphabet agrees with nothing, got {agreement}"
        );
    }
}

#[sqlx::test(migrator = "clearcut_store::MIGRATOR")]
#[ignore = "needs the real worker with OCR and a database; run by the CI contract job"]
async fn a_pdf_whose_digits_are_shifted_is_not_trusted_either(pool: PgPool) {
    let bytes = pdf(DIGITS_SHIFTED);
    // Ground truth: every digit in the text layer is the next digit, so every amount is wrong
    // while the text is still perfectly good text.
    let found = words_of(&text_layer(&bytes, 1).await);
    let cipher = DIGITS_SHIFTED.unwrap();
    assert!(
        found.contains(&cipher.text("Total amount due R1725.00")),
        "{found}"
    );
    assert!(found.contains("Total amount due R2836.11"), "{found}");

    let analysed = analyse(&pool, bytes).await;
    assert_eq!(analysed.ocr, "ran");
    assert_eq!(
        analysed.ocr_pages, 2,
        "page one disagreed, so the rest was read by OCR too"
    );

    // Page one has the amounts. The text measures cannot see this lie; the OCR agreement does,
    // and the page is never read from the text layer alone.
    let one = &analysed.reads[0];
    assert_ne!(one.route(), ReadRoute::TextAndImage, "{one:?}");
    let agreement = one.ocr_agreement().expect("compared with OCR");
    assert!(agreement < 850, "got {agreement}");

    // Page two has no digits at all, so its text layer is right and is trusted: the check is
    // per page, not a verdict on the whole file.
    let two = &analysed.reads[1];
    assert_eq!(two.route(), ReadRoute::TextAndImage, "{two:?}");
    assert!(two.ocr_agreement().expect("compared with OCR") >= 850);
}
