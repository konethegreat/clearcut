// Written by Kone Tshivhinda & Claude
use super::*;
use crate::check::{check_set, Problem};
use crate::hash::sha256_hex;
use crate::label::{Label, LabelProblem};
use std::fs;
use tempfile::TempDir;
use time::macros::date;

struct Setup {
    dir: TempDir,
    gold: std::path::PathBuf,
}

impl Setup {
    fn new() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let gold = dir.path().join("gold");
        Setup { dir, gold }
    }

    fn gold(&self) -> std::path::PathBuf {
        self.gold.clone()
    }

    /// A source document with these bytes, named `name`.
    fn source(&self, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = self.dir.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn request<'a>(&'a self, document: &'a Path) -> NewCase<'a> {
        NewCase {
            gold: &self.gold,
            document,
            split: Split::Dev,
            tier: Tier::Consented,
            labelled_by: "KT",
            pages: 2,
            consent_ref: Some("C-017"),
            retain_until: Some(date!(2031 - 03 - 31)),
        }
    }
}

fn label_of(gold: &Path, id: &str) -> Label {
    Label::parse(&fs::read(gold.join(id).join("label.json")).unwrap()).unwrap()
}

#[test]
fn the_first_case_is_g_0001_and_holds_a_copy_of_the_document() {
    let setup = Setup::new();
    let source = setup.source("Scan 12 (client).PDF", b"%PDF-1.4 hello");
    let id = create(&setup.request(&source)).unwrap();
    assert_eq!(id, "g-0001");
    let folder = setup.gold().join("g-0001");
    assert_eq!(fs::read(folder.join("document.pdf")).unwrap(), b"%PDF-1.4 hello");
    // The original is untouched, and the file name it had is nowhere in the case.
    assert!(source.exists());
    let names: Vec<String> = fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2);
    assert!(!fs::read_to_string(folder.join("label.json"))
        .unwrap()
        .contains("Scan"));
}

#[test]
fn the_label_records_what_was_asked_and_the_documents_real_hash() {
    let setup = Setup::new();
    let source = setup.source("a.pdf", b"some bytes");
    create(&setup.request(&source)).unwrap();
    let label = label_of(&setup.gold(), "g-0001");
    assert_eq!(label.format, 1);
    assert_eq!(label.case, "g-0001");
    assert_eq!(label.document.file, "document.pdf");
    assert_eq!(label.document.sha256, sha256_hex(b"some bytes"));
    assert_eq!(label.document.pages, 2);
    assert_eq!(label.split, Split::Dev);
    assert_eq!(label.provenance.tier, Tier::Consented);
    assert_eq!(label.provenance.consent_ref.as_deref(), Some("C-017"));
    assert_eq!(label.provenance.retain_until, Some(date!(2031 - 03 - 31)));
    assert_eq!(label.provenance.labelled_by, "KT");
    assert!(label.provenance.reviewed_by.is_none());
    assert!(label.tags.is_empty() && label.expect.findings.is_empty());
}

#[test]
fn a_real_document_starts_out_not_anonymised_and_a_synthetic_one_does() {
    let setup = Setup::new();
    let source = setup.source("a.pdf", b"x");
    create(&setup.request(&source)).unwrap();
    assert!(!label_of(&setup.gold(), "g-0001").provenance.anonymised);
    let request = NewCase {
        tier: Tier::Synthetic,
        consent_ref: None,
        retain_until: None,
        ..setup.request(&source)
    };
    create(&request).unwrap();
    assert!(label_of(&setup.gold(), "g-0002").provenance.anonymised);
}

#[test]
fn ids_continue_after_the_highest_existing_case_and_ignore_other_folders() {
    let setup = Setup::new();
    let source = setup.source("a.pdf", b"x");
    for existing in ["g-0003", "g-0007", "g-0002", "notes", "g-12", "G-0099"] {
        fs::create_dir_all(setup.gold().join(existing)).unwrap();
    }
    fs::write(setup.gold().join("g-9999"), "a file, not a case").unwrap();
    assert_eq!(create(&setup.request(&source)).unwrap(), "g-0008");
    assert_eq!(create(&setup.request(&source)).unwrap(), "g-0009");
}

#[test]
fn the_gold_folder_is_created_when_missing_but_must_otherwise_be_a_folder() {
    let setup = Setup::new();
    let source = setup.source("a.pdf", b"x");
    assert!(!setup.gold().exists());
    create(&setup.request(&source)).unwrap();
    assert!(setup.gold().is_dir());

    let other = Setup::new();
    let source = other.source("a.pdf", b"x");
    fs::write(other.gold(), "a file").unwrap();
    assert!(matches!(
        create(&other.request(&source)),
        Err(NewCaseError::Io(_))
    ));
}

#[test]
fn the_extension_is_kept_in_lower_case_and_must_be_plain() {
    let setup = Setup::new();
    for (name, stored) in [
        ("a.PDF", "document.pdf"),
        ("b.JpEg", "document.jpeg"),
        ("c.tiff", "document.tiff"),
        ("d.xml", "document.xml"),
        ("e.abcde", "document.abcde"),
    ] {
        let source = setup.source(name, b"x");
        let id = create(&setup.request(&source)).unwrap();
        // Compare the names as stored (a case-insensitive file system would match either spelling).
        let names: Vec<String> = fs::read_dir(setup.gold().join(&id))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&stored.to_string()), "{name}: {names:?}");
    }
}

#[test]
fn an_extension_that_is_not_plain_is_refused() {
    let setup = Setup::new();
    for name in ["noextension", "x.", "x.toolongext", "x.abcdef", "x.p-f", ".pdf"] {
        let source = setup.source(name, b"x");
        assert_eq!(
            create(&setup.request(&source)),
            Err(NewCaseError::BadExtension),
            "{name}"
        );
    }
}

#[test]
fn nothing_is_left_behind_when_a_case_is_refused() {
    let setup = Setup::new();
    let source = setup.source("noextension", b"x");
    let _ = create(&setup.request(&source));
    assert!(!setup.gold().join("g-0001").exists());
}

#[test]
fn the_source_must_be_a_real_non_empty_file() {
    let setup = Setup::new();
    let missing = setup.dir.path().join("missing.pdf");
    assert!(matches!(
        create(&setup.request(&missing)),
        Err(NewCaseError::Source(_))
    ));

    let folder = setup.dir.path().join("folder.pdf");
    fs::create_dir(&folder).unwrap();
    assert!(matches!(
        create(&setup.request(&folder)),
        Err(NewCaseError::Source(_))
    ));

    let empty = setup.source("empty.pdf", b"");
    assert_eq!(create(&setup.request(&empty)), Err(NewCaseError::EmptyDocument));
    assert!(!setup.gold().join("g-0001").exists());
}

#[cfg(unix)]
#[test]
fn a_link_is_not_followed() {
    let setup = Setup::new();
    let real = setup.source("real.pdf", b"x");
    let link = setup.dir.path().join("link.pdf");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert!(matches!(
        create(&setup.request(&link)),
        Err(NewCaseError::Source(_))
    ));
}

#[test]
fn the_details_are_checked() {
    let setup = Setup::new();
    let source = setup.source("a.pdf", b"x");
    let base = || setup.request(&source);
    for (request, argument) in [
        (NewCase { pages: 0, ..base() }, "pages"),
        (
            NewCase {
                pages: 10_001,
                ..base()
            },
            "pages",
        ),
        (
            NewCase {
                labelled_by: "  ",
                ..base()
            },
            "labelled-by",
        ),
        (
            NewCase {
                labelled_by: "a\nb",
                ..base()
            },
            "labelled-by",
        ),
        (
            NewCase {
                labelled_by: &"x".repeat(65),
                ..base()
            },
            "labelled-by",
        ),
        (
            NewCase {
                consent_ref: Some(" "),
                ..base()
            },
            "consent-ref",
        ),
    ] {
        assert_eq!(
            create(&request),
            Err(NewCaseError::BadArgument(argument)),
            "{argument}"
        );
    }
    assert!(!setup.gold().join("g-0001").exists());
    assert!(create(&NewCase {
        pages: 10_000,
        labelled_by: &"x".repeat(64),
        ..base()
    })
    .is_ok());
}

#[test]
fn a_new_case_is_accepted_in_form_but_not_finished() {
    let setup = Setup::new();
    let source = setup.source("a.pdf", b"%PDF-1.4 hello");
    create(&setup.request(&source)).unwrap();
    let report = check_set(&setup.gold()).unwrap();
    let problems: Vec<&Problem> = report.problems.iter().map(|p| &p.problem).collect();
    // The hash, page citations, folder contents and consent details are right...
    assert!(!problems.iter().any(|p| matches!(
        p,
        Problem::HashMismatch
            | Problem::DocumentMissing
            | Problem::UnexpectedEntries { .. }
            | Problem::Parse(_)
    )));
    // ...but it is loudly unfinished: not anonymised yet, and no lines read.
    assert!(
        problems.contains(&&Problem::Label(LabelProblem::NotAnonymised)),
        "{problems:?}"
    );
    assert!(
        problems.iter().any(|p| matches!(p, Problem::FindingsDiffer(_))),
        "{problems:?}"
    );
}

#[test]
fn the_error_text_never_includes_a_name_from_the_command() {
    let setup = Setup::new();
    let source = setup.source("SecretClient.pdf", b"");
    let error = create(&NewCase {
        labelled_by: "SecretPerson",
        ..setup.request(&source)
    })
    .unwrap_err();
    assert!(!error.to_string().contains("Secret"), "{error}");
    let error = create(&NewCase {
        pages: 0,
        labelled_by: "SecretPerson",
        ..setup.request(&source)
    })
    .unwrap_err();
    assert!(!error.to_string().contains("Secret"), "{error}");
}
