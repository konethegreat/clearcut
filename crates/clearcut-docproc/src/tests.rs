// Written by Kone Tshivhinda & Claude
use super::*;
use bytes::Bytes;
use clearcut_core::jobs::FailureKind;

fn pdf_inspection(pages: u32, with_text: u32) -> Inspection {
    Inspection::new(DocumentKind::Pdf, pages, with_text).unwrap()
}

// ---------- what an inspection says ----------

#[test]
fn an_inspection_reports_pages_and_whether_there_is_a_text_layer() {
    let mixed = pdf_inspection(4, 2);
    assert_eq!(mixed.kind(), DocumentKind::Pdf);
    assert_eq!(mixed.pages(), 4);
    assert_eq!(mixed.pages_with_text(), 2);
    assert!(mixed.has_text_layer());

    let scan = pdf_inspection(3, 0);
    assert!(!scan.has_text_layer());
    assert!(pdf_inspection(9, 1).has_text_layer());
    assert!(pdf_inspection(1, 1).has_text_layer());
}

#[test]
fn text_pages_can_never_be_more_than_pages() {
    assert!(Inspection::new(DocumentKind::Pdf, 3, 3).is_ok());
    assert!(Inspection::new(DocumentKind::Pdf, 0, 0).is_ok());
    assert!(Inspection::new(DocumentKind::Pdf, 3, 4).is_err());
    assert!(Inspection::new(DocumentKind::Pdf, 0, 1).is_err());
}

#[test]
fn the_kinds_and_their_names() {
    assert_eq!(DocumentKind::Pdf.media_type(), "application/pdf");
    assert_eq!(DocumentKind::parse("application/pdf"), Some(DocumentKind::Pdf));
    for bad in [
        "",
        "APPLICATION/PDF",
        " application/pdf",
        "image/png",
        "application/pdf; x=1",
    ] {
        assert_eq!(DocumentKind::parse(bad), None, "{bad:?}");
    }
}

// ---------- how errors map onto the job queue ----------

#[test]
fn each_error_maps_to_the_queues_failure_kind() {
    use DocumentProblem::*;
    let table: Vec<(ProcessError, FailureKind)> = vec![
        (ProcessError::Unavailable, FailureKind::ProviderUnavailable),
        (ProcessError::Auth, FailureKind::ProviderUnavailable),
        (
            ProcessError::Unexpected { status: 404 },
            FailureKind::ProviderUnavailable,
        ),
        (
            ProcessError::Unexpected { status: 429 },
            FailureKind::ProviderUnavailable,
        ),
        (ProcessError::Timeout, FailureKind::Transient),
        (ProcessError::Interrupted, FailureKind::Transient),
        (ProcessError::Server { status: 500 }, FailureKind::Transient),
        (ProcessError::Server { status: 504 }, FailureKind::Transient),
        (ProcessError::BadResponse("not json"), FailureKind::Transient),
        (ProcessError::Document(Empty), FailureKind::NeedsReview),
        (
            ProcessError::Document(TooLarge { max_bytes: 1 }),
            FailureKind::NeedsReview,
        ),
        (ProcessError::Document(UnsupportedType), FailureKind::NeedsReview),
        (ProcessError::Document(Encrypted), FailureKind::NeedsReview),
        (ProcessError::Document(Unreadable), FailureKind::NeedsReview),
        (
            ProcessError::Document(TooManyPages { max_pages: 5 }),
            FailureKind::NeedsReview,
        ),
    ];
    for (error, kind) in table {
        assert_eq!(error.failure_kind(), kind, "{error}");
    }
}

#[test]
fn error_messages_are_plain_and_name_the_limit_when_there_is_one() {
    assert_eq!(
        ProcessError::Unavailable.to_string(),
        "the document worker could not be reached"
    );
    assert_eq!(
        ProcessError::Auth.to_string(),
        "the document worker refused our credentials"
    );
    assert_eq!(ProcessError::Timeout.to_string(), "the document worker timed out");
    assert_eq!(
        ProcessError::Interrupted.to_string(),
        "the connection to the document worker was interrupted"
    );
    assert_eq!(
        ProcessError::Server { status: 503 }.to_string(),
        "the document worker failed (HTTP 503)"
    );
    assert_eq!(
        ProcessError::Unexpected { status: 404 }.to_string(),
        "the document worker answered unexpectedly (HTTP 404)"
    );
    assert_eq!(
        ProcessError::BadResponse("missing field").to_string(),
        "the document worker's answer was not understood: missing field"
    );
    assert_eq!(
        ProcessError::Document(DocumentProblem::TooManyPages { max_pages: 500 }).to_string(),
        "the document has more than 500 pages"
    );
    assert_eq!(
        ProcessError::Document(DocumentProblem::TooLarge { max_bytes: 100 }).to_string(),
        "the document is larger than 100 bytes"
    );
    assert_eq!(
        ProcessError::Document(DocumentProblem::Encrypted).to_string(),
        "the document is encrypted"
    );
    assert_eq!(
        ProcessError::Document(DocumentProblem::Unreadable).to_string(),
        "the document could not be read"
    );
    assert_eq!(
        ProcessError::Document(DocumentProblem::UnsupportedType).to_string(),
        "the document is not a supported type"
    );
    assert_eq!(
        ProcessError::Document(DocumentProblem::Empty).to_string(),
        "the document is empty"
    );
    let boxed: Box<dyn std::error::Error> = Box::new(ProcessError::Timeout);
    assert!(!boxed.to_string().is_empty());
}

#[test]
fn only_a_problem_with_the_document_itself_needs_a_person() {
    assert!(ProcessError::Document(DocumentProblem::Encrypted).is_about_the_document());
    assert!(!ProcessError::Unavailable.is_about_the_document());
    assert!(!ProcessError::Server { status: 500 }.is_about_the_document());
}

// ---------- the scripted fake ----------

async fn ask<P: DocumentProcessor>(
    processor: &P,
    document: &'static [u8],
) -> Result<Inspection, ProcessError> {
    processor.inspect(Bytes::from_static(document)).await
}

#[tokio::test]
async fn the_fake_gives_its_scripted_answers_in_order_then_says_it_has_none() {
    let fake = FakeWorker::scripted(vec![
        Ok(pdf_inspection(1, 1)),
        Err(ProcessError::Timeout),
        Ok(pdf_inspection(5, 0)),
    ]);
    assert_eq!(ask(&fake, b"a").await.unwrap().pages(), 1);
    assert!(matches!(ask(&fake, b"b").await, Err(ProcessError::Timeout)));
    assert_eq!(ask(&fake, b"c").await.unwrap().pages(), 5);
    match ask(&fake, b"d").await {
        Err(ProcessError::Unexpected { status: 0 }) => {}
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn the_fake_remembers_every_document_it_was_given() {
    let fake = FakeWorker::scripted(vec![Ok(pdf_inspection(1, 1)), Ok(pdf_inspection(2, 2))]);
    assert_eq!(fake.call_count(), 0);
    ask(&fake, b"first document").await.unwrap();
    ask(&fake, b"second").await.unwrap();
    assert_eq!(fake.call_count(), 2);
    assert_eq!(
        fake.documents(),
        [
            Bytes::from_static(b"first document"),
            Bytes::from_static(b"second")
        ]
    );
}

#[tokio::test]
async fn the_fake_counts_a_call_even_when_it_has_nothing_to_say() {
    let fake = FakeWorker::scripted(vec![]);
    let _ = ask(&fake, b"x").await;
    assert_eq!(fake.call_count(), 1);
}

#[tokio::test]
async fn the_fake_can_answer_from_the_document() {
    let fake = FakeWorker::answering(|document| Ok(pdf_inspection(document.len() as u32, 0)));
    assert_eq!(ask(&fake, b"12345").await.unwrap().pages(), 5);
    assert_eq!(ask(&fake, b"12").await.unwrap().pages(), 2);
}

#[tokio::test]
async fn clones_of_the_fake_share_one_script_and_one_record() {
    let fake = FakeWorker::scripted(vec![Ok(pdf_inspection(1, 1)), Ok(pdf_inspection(2, 2))]);
    let other = fake.clone();
    ask(&fake, b"a").await.unwrap();
    assert_eq!(ask(&other, b"b").await.unwrap().pages(), 2);
    assert_eq!((fake.call_count(), other.call_count()), (2, 2));
}
