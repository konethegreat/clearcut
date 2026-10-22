// Written by Kone Tshivhinda & Claude
use super::*;
use crate::label::tests::base;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

const GITIGNORE: &str = "/target\n/gold/\n/gold-*/\n";

/// A made-up repository in a temporary folder.
struct Repo {
    dir: TempDir,
}

impl Repo {
    /// A clean repository: the config approving two folders, a `.gitignore`
    /// with the gold-folder entries, and some ordinary files.
    fn new() -> Repo {
        let repo = Repo {
            dir: tempfile::tempdir().unwrap(),
        };
        repo.write(
            CONFIG_FILE,
            r#"{ "approved_folders": ["fixtures/docs", "samples"] }"#,
        );
        repo.write(".gitignore", GITIGNORE);
        repo.write("README.md", "# hello");
        repo.write("src/main.rs", "fn main() {}");
        repo
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn write(&self, path: &str, content: impl AsRef<[u8]>) {
        let full = self.root().join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, content).unwrap();
    }

    fn guard(&self) -> GuardReport {
        guard(self.root()).unwrap()
    }

    fn violations(&self) -> Vec<String> {
        self.guard().violations.iter().map(ToString::to_string).collect()
    }
}

fn label_with_tier(tier: &str) -> String {
    let mut value = base();
    value["provenance"]["tier"] = json!(tier);
    value.to_string()
}

// ---- a clean repository ----

#[test]
fn a_repository_with_no_documents_is_clean_and_files_are_counted() {
    let report = Repo::new().guard();
    assert!(report.is_clean(), "{:?}", report.violations);
    assert_eq!(report.files_scanned, 4);
}

// ---- documents ----

#[test]
fn a_document_anywhere_outside_the_approved_folders_is_a_violation() {
    for extension in [
        "pdf", "png", "jpg", "jpeg", "tif", "tiff", "gif", "bmp", "webp", "heic", "heif", "doc", "docx",
        "xls", "xlsx", "odt", "ods", "zip", "tar", "gz", "7z", "eml", "msg",
    ] {
        let repo = Repo::new();
        repo.write(&format!("docs/invoice.{extension}"), "x");
        assert_eq!(repo.guard().violations.len(), 1, "{extension}");
        assert!(repo.violations()[0].starts_with("docs/invoice."), "{extension}");
    }
}

#[test]
fn extensions_are_matched_without_regard_to_case() {
    for name in ["A.PDF", "b.Pdf", "c.JPG", "d.TiFf"] {
        let repo = Repo::new();
        repo.write(&format!("x/{name}"), "x");
        assert_eq!(repo.guard().violations.len(), 1, "{name}");
    }
}

#[test]
fn only_the_last_extension_counts_and_other_files_are_fine() {
    let repo = Repo::new();
    for name in [
        "notes.pdf.txt",
        "pdf",
        "report.pdfx",
        "data.json",
        "image.svg",
        "table.csv",
        "invoice.xml",
    ] {
        repo.write(&format!("x/{name}"), "plain text");
    }
    assert!(repo.guard().is_clean(), "{:?}", repo.violations());
}

#[test]
fn documents_are_welcome_inside_an_approved_folder_at_any_depth() {
    let repo = Repo::new();
    repo.write("fixtures/docs/a.pdf", "x");
    repo.write("fixtures/docs/deeper/still/b.png", "x");
    repo.write("samples/g-0001/document.pdf", "x");
    assert!(repo.guard().is_clean(), "{:?}", repo.violations());
}

#[test]
fn an_approved_folder_means_that_folder_and_not_one_that_merely_starts_the_same() {
    let repo = Repo::new();
    repo.write("samples-private/a.pdf", "x");
    repo.write("fixtures/docs-extra/b.pdf", "x");
    repo.write("fixtures/a.pdf", "x");
    repo.write("elsewhere/samples/c.pdf", "x");
    assert_eq!(repo.guard().violations.len(), 4, "{:?}", repo.violations());
}

#[test]
fn a_document_renamed_to_something_harmless_is_found_by_its_first_bytes() {
    let repo = Repo::new();
    repo.write("x/notes.txt", b"%PDF-1.7\n...");
    repo.write("x/photo.dat", b"\x89PNG\r\n\x1a\n");
    repo.write("x/scan.bin", b"\xff\xd8\xff\xe0\x00\x10JFIF");
    repo.write("x/anim.bin", b"GIF89a....");
    repo.write("x/tiff1.bin", b"II*\x00....");
    repo.write("x/tiff2.bin", b"MM\x00*....");
    repo.write("x/bundle.dat", b"PK\x03\x04....");
    let violations = repo.violations();
    assert_eq!(violations.len(), 7, "{violations:?}");
    assert!(
        violations.iter().all(|v| v.contains("looks like a document")),
        "{violations:?}"
    );
}

#[test]
fn ordinary_text_and_very_short_files_are_not_mistaken_for_documents() {
    let repo = Repo::new();
    repo.write("x/empty.txt", "");
    repo.write("x/one.txt", "%");
    repo.write("x/almost.txt", "%PDF"); // not the full magic
    repo.write("x/text.txt", "PK is a pair of letters; GIF is a format");
    repo.write("x/binary.dat", b"\x00\x01\x02\x03\x04\x05\x06\x07");
    assert!(repo.guard().is_clean(), "{:?}", repo.violations());
}

#[test]
fn the_first_bytes_of_files_in_approved_folders_are_not_questioned() {
    let repo = Repo::new();
    repo.write("samples/g-0001/blob.bin", b"%PDF-1.4");
    assert!(repo.guard().is_clean());
}

// ---- labels ----

#[test]
fn a_label_outside_the_approved_folders_is_a_violation() {
    let repo = Repo::new();
    repo.write("anywhere/g-0001/label.json", label_with_tier("synthetic"));
    repo.write("other/Label.JSON", "{}");
    let violations = repo.violations();
    assert_eq!(violations.len(), 2, "{violations:?}");
    assert!(
        violations.iter().all(|v| v.contains("gold-set label")),
        "{violations:?}"
    );
}

#[test]
fn a_synthetic_label_in_an_approved_folder_is_fine() {
    let repo = Repo::new();
    repo.write("samples/g-0001/label.json", label_with_tier("synthetic"));
    assert!(repo.guard().is_clean(), "{:?}", repo.violations());
}

#[test]
fn a_real_case_cannot_hide_in_an_approved_folder() {
    let repo = Repo::new();
    repo.write("samples/g-0001/label.json", label_with_tier("consented"));
    repo.write("samples/g-0002/label.json", label_with_tier("own"));
    repo.write("samples/g-0003/label.json", "{ nope");
    repo.write("samples/g-0004/label.json", r#"{ "surprise": true }"#);
    let violations = repo.violations();
    assert_eq!(violations.len(), 4, "{violations:?}");
    assert!(
        violations[0].contains("not synthetic") && violations[1].contains("not synthetic"),
        "{violations:?}"
    );
    assert!(
        violations[2].contains("cannot be read") && violations[3].contains("cannot be read"),
        "{violations:?}"
    );
}

// ---- places that are not looked into ----

#[test]
fn build_output_dependencies_and_git_internals_are_skipped() {
    let repo = Repo::new();
    for folder in [
        "target",
        ".git",
        "node_modules",
        ".venv",
        "services/worker/.venv",
        "a/b/__pycache__",
        "dist",
    ] {
        repo.write(&format!("{folder}/hidden.pdf"), "x");
    }
    let report = repo.guard();
    assert!(report.is_clean(), "{:?}", report.violations);
    assert_eq!(report.files_scanned, 4);
}

#[cfg(unix)]
#[test]
fn links_are_never_followed_but_a_link_named_like_a_document_counts() {
    let repo = Repo::new();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.pdf"), "x").unwrap();
    std::os::unix::fs::symlink(outside.path(), repo.root().join("linked")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.pdf"),
        repo.root().join("shortcut.pdf"),
    )
    .unwrap();
    assert_eq!(
        repo.violations(),
        ["shortcut.pdf: a document file outside the approved fixture folders"]
    );
}

// ---- .gitignore ----

#[test]
fn the_gitignore_must_keep_the_gold_folders_out() {
    let repo = Repo::new();
    repo.write(".gitignore", "/target\n");
    assert_eq!(
        repo.violations(),
        [
            ".gitignore: the entry /gold/ is missing, so a gold folder inside the repository could be committed",
            ".gitignore: the entry /gold-*/ is missing, so a gold folder inside the repository could be committed",
        ]
    );
    let repo = Repo::new();
    fs::remove_file(repo.root().join(".gitignore")).unwrap();
    assert_eq!(repo.guard().violations.len(), 2);
}

#[test]
fn gitignore_entries_may_have_spaces_and_comments_around_them() {
    let repo = Repo::new();
    repo.write(".gitignore", "# private gold sets\n  /gold/  \n\n/gold-*/\r\n");
    assert!(repo.guard().is_clean(), "{:?}", repo.violations());
}

#[test]
fn a_commented_out_entry_does_not_count() {
    let repo = Repo::new();
    repo.write(".gitignore", "#/gold/\n/gold-*/\n");
    assert_eq!(repo.guard().violations.len(), 1);
}

// ---- reporting ----

#[test]
fn violations_are_listed_in_path_order_with_forward_slashes() {
    let repo = Repo::new();
    repo.write("z/last.pdf", "x");
    repo.write("a/b/first.png", "x");
    repo.write("m/middle.jpg", "x");
    let paths: Vec<String> = repo
        .violations()
        .iter()
        .map(|v| v.split(':').next().unwrap().to_string())
        .collect();
    assert_eq!(paths, ["a/b/first.png", "m/middle.jpg", "z/last.pdf"]);
}

#[test]
fn nothing_inside_a_file_is_ever_printed() {
    let repo = Repo::new();
    repo.write("x/notes.txt", b"%PDF-1.4 SECRETCONTENT");
    repo.write(
        "samples/g-0001/label.json",
        r#"{ "case": "SECRETCASE", "nope": "SECRETVALUE" }"#,
    );
    let text = repo.violations().join("\n");
    assert!(!text.contains("SECRET"), "{text}");
}

// ---- the configuration ----

#[test]
fn the_config_file_is_required() {
    let repo = Repo::new();
    fs::remove_file(repo.root().join(CONFIG_FILE)).unwrap();
    assert_eq!(guard(repo.root()).unwrap_err(), GuardError::ConfigMissing);
}

#[test]
fn a_config_with_no_approved_folders_is_allowed_and_means_no_documents_at_all() {
    let repo = Repo::new();
    repo.write(CONFIG_FILE, r#"{ "approved_folders": [] }"#);
    repo.write("samples/a.pdf", "x");
    assert_eq!(repo.guard().violations.len(), 1);
}

#[test]
fn a_malformed_config_is_refused_without_echoing_it() {
    for text in [
        "{ nope",
        "",
        r#"{ "approved_folders": "samples" }"#,
        r#"{ "approved": ["samples"] }"#,
        r#"{ "approved_folders": [], "secretkey": 1 }"#,
        r#"{}"#,
    ] {
        let repo = Repo::new();
        repo.write(CONFIG_FILE, text);
        let error = guard(repo.root()).unwrap_err();
        assert!(matches!(error, GuardError::ConfigInvalid(_)), "{text}: {error:?}");
        assert!(!error.to_string().contains("secretkey"), "{error}");
    }
}

#[test]
fn approved_folders_must_be_plain_relative_paths_inside_the_repository() {
    for bad in [
        "", ".", "..", "/abs", "a/../b", "../a", "a\\b", "C:/x", "a//b", "./a", "a/./b", " ", "a/",
    ] {
        let text = json!({ "approved_folders": ["fine", bad] }).to_string();
        let error = GuardConfig::parse(text.as_bytes()).unwrap_err();
        assert_eq!(error, GuardError::BadApprovedFolder { index: 1 }, "{bad:?}");
    }
    let ok =
        GuardConfig::parse(br#"{ "approved_folders": ["a", "b/c", "dir-with.dots_and-dashes"] }"#).unwrap();
    assert_eq!(ok.approved_folders, ["a", "b/c", "dir-with.dots_and-dashes"]);
}

#[test]
fn a_repository_folder_that_cannot_be_read_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        guard(&dir.path().join("nope")),
        Err(GuardError::ConfigMissing | GuardError::Unreadable(_))
    ));
}

// ---- this very repository ----

#[test]
fn this_repository_passes_its_own_guard() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let report = guard(&root).unwrap();
    assert!(
        report.is_clean(),
        "{:#?}",
        report
            .violations
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
    assert!(report.files_scanned > 100);
}

// ---- inside a git working copy ----

fn git(repo: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[test]
fn in_a_git_working_copy_only_what_git_would_commit_is_judged() {
    let repo = Repo::new();
    if !git(repo.root(), &["init", "-q"]) {
        return; // no git here; the plain walk is tested above
    }
    repo.write(".gitignore", "/gold/\n/gold-*/\nscratch/\n*.local.pdf\n");
    repo.write("scratch/old.pdf", "x");
    repo.write("mine.local.pdf", "x");
    assert!(
        repo.guard().is_clean(),
        "ignored files are not findings: {:?}",
        repo.violations()
    );

    // A document git would commit.
    repo.write("oops.pdf", "x");
    assert_eq!(
        repo.violations(),
        ["oops.pdf: a document file outside the approved fixture folders"]
    );

    // Forcing an ignored file in makes it count.
    assert!(git(repo.root(), &["add", "-f", "scratch/old.pdf"]));
    assert_eq!(repo.guard().violations.len(), 2);

    // A tracked file deleted from the working copy no longer counts.
    assert!(git(repo.root(), &["add", "oops.pdf"]));
    fs::remove_file(repo.root().join("oops.pdf")).unwrap();
    assert_eq!(
        repo.violations(),
        ["scratch/old.pdf: a document file outside the approved fixture folders"]
    );
}

#[test]
fn a_gold_folder_inside_a_working_copy_is_ignored_by_git_so_it_cannot_be_committed_by_accident() {
    let repo = Repo::new();
    if !git(repo.root(), &["init", "-q"]) {
        return;
    }
    repo.write("gold/g-0001/document.pdf", "x");
    repo.write("gold/g-0001/label.json", label_with_tier("consented"));
    repo.write("gold-private/g-0002/document.pdf", "x");
    assert!(repo.guard().is_clean(), "{:?}", repo.violations());
    assert!(
        !git(repo.root(), &["add", "gold"]),
        "git itself should refuse to add an ignored folder"
    );
}

// ---- more formats, by their first bytes ----

#[test]
fn more_document_signatures_are_recognised() {
    let bmp = {
        let mut head = vec![b'B', b'M', 0x46, 0, 0, 0, 0, 0, 0, 0, 0x36, 0, 0, 0, 40, 0];
        head.extend_from_slice(&[0; 8]);
        head
    };
    let webp = b"RIFF\x24\x00\x00\x00WEBPVP8 ".to_vec();
    for (name, head) in [
        ("old-office", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1....".to_vec()),
        ("gzip", b"\x1f\x8b\x08\x00".to_vec()),
        ("seven-zip", b"7z\xbc\xaf\x27\x1c\x00\x04".to_vec()),
        ("bitmap", bmp),
        ("webp", webp),
        ("gif87", b"GIF87a..".to_vec()),
    ] {
        assert!(starts_like_a_document(&head), "{name}");
        let repo = Repo::new();
        repo.write("x/data.bin", &head);
        assert_eq!(repo.guard().violations.len(), 1, "{name}");
    }
}

#[test]
fn text_and_other_formats_that_merely_look_similar_are_left_alone() {
    for (name, head) in [
        (
            "text starting with BM",
            b"BMW and BMI, a note about cars and health".to_vec(),
        ),
        ("just BM", b"BM".to_vec()),
        (
            "BM with a wrong header size",
            b"BM\x46\x00\x00\x00\x00\x00\x00\x00\x36\x00\x00\x00\x99\x00....".to_vec(),
        ),
        ("BM with reserved bytes set", b"BMF      6   ( ....".to_vec()),
        ("a bitmap header cut one byte short", b"BMF       6   (".to_vec()),
        ("wav audio", b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec()),
        ("riff too short", b"RIFF\x24\x00\x00\x00WEB".to_vec()),
        ("7z lookalike", b"7zip is a tool".to_vec()),
        ("plain text", b"hello world, nothing to see".to_vec()),
        ("empty", Vec::new()),
    ] {
        assert!(!starts_like_a_document(&head), "{name}");
    }
}

#[cfg(unix)]
#[test]
fn a_link_is_judged_by_its_name_alone_and_never_read_through() {
    let repo = Repo::new();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("real.dat"), b"%PDF-1.4 secret").unwrap();
    std::os::unix::fs::symlink(outside.path().join("real.dat"), repo.root().join("shortcut.dat")).unwrap();
    assert!(repo.guard().is_clean(), "{:?}", repo.violations());
}
