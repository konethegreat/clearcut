// Written by Kone Tshivhinda & Claude
//! A tiny PDF writer, just enough to turn lines of text into a document with a
//! real text layer, so the synthetic sample set holds files the pipeline can
//! actually read. It writes one built-in font (Helvetica) and nothing else:
//! no images, no compression, no dates, no random ids, so the same input
//! always gives the same bytes.

use std::fmt::Write as _;

const PAGE_WIDTH: u32 = 595; // A4, in points
const PAGE_HEIGHT: u32 = 842;
const LEFT: u32 = 50;
const TOP: u32 = 792;
const LINE_HEIGHT: u32 = 16;

/// How many lines fit on one page.
pub const LINES_PER_PAGE: usize = ((TOP - 50) / LINE_HEIGHT) as usize;

/// A PDF with one page per entry of `pages`, each a list of text lines.
/// Characters outside printable ASCII are written as `?`; lines beyond what
/// fits on a page are dropped (callers keep to [`LINES_PER_PAGE`]).
pub fn simple_pdf(pages: &[Vec<String>]) -> Vec<u8> {
    // Object numbers: 1 catalog, 2 page tree, 3 font, then a page object and a
    // content stream for each page.
    let page_count = pages.len().max(1);
    let first_page_object = 4;
    let mut objects: Vec<String> = Vec::new();

    objects.push("<< /Type /Catalog /Pages 2 0 R >>".to_string());
    let kids: Vec<String> = (0..page_count)
        .map(|i| format!("{} 0 R", first_page_object + 2 * i))
        .collect();
    objects.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {page_count} >>",
        kids.join(" ")
    ));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());

    let empty = Vec::new();
    for i in 0..page_count {
        let content_object = first_page_object + 2 * i + 1;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH} {PAGE_HEIGHT}] \
             /Resources << /Font << /F1 3 0 R >> >> /Contents {content_object} 0 R >>"
        ));
        let lines = pages.get(i).unwrap_or(&empty);
        let mut stream = String::from("BT\n/F1 10 Tf\n");
        let _ = writeln!(stream, "{LINE_HEIGHT} TL");
        let _ = writeln!(stream, "{LEFT} {TOP} Td");
        for line in lines.iter().take(LINES_PER_PAGE) {
            let _ = writeln!(stream, "({}) Tj T*", escape(line));
        }
        stream.push_str("ET\n");
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}endstream",
            stream.len()
        ));
    }

    // Header, then each object remembering where it starts, then the table of
    // those starting points (the cross-reference table) and the trailer.
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        let _ = write!(out, "{} 0 obj\n{body}\nendobj\n", index + 1);
    }
    let xref_at = out.len();
    let _ = writeln!(out, "xref\n0 {}", objects.len() + 1);
    out.push_str("0000000000 65535 f \n");
    for offset in offsets {
        let _ = writeln!(out, "{offset:010} 00000 n ");
    }
    let _ = write!(
        out,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
        objects.len() + 1
    );
    out.into_bytes()
}

/// Escapes a line for a PDF string: backslash and parentheses get a
/// backslash, anything but printable ASCII becomes `?`.
fn escape(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        match c {
            '\\' | '(' | ')' => {
                out.push('\\');
                out.push(c);
            }
            ' '..='~' => out.push(c),
            _ => out.push('?'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).expect("the writer only emits ASCII")
    }

    fn page(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| (*line).to_string()).collect()
    }

    #[test]
    fn has_the_marks_of_a_pdf() {
        let pdf = text(&simple_pdf(&[page(&["Hello"])]));
        assert!(pdf.starts_with("%PDF-1.4\n"));
        assert!(pdf.ends_with("%%EOF\n"));
        assert!(pdf.contains("/Type /Catalog"));
        assert!(pdf.contains("/Count 1"));
        assert!(pdf.contains("(Hello) Tj"));
    }

    #[test]
    fn every_cross_reference_entry_points_at_its_object() {
        let bytes = simple_pdf(&[page(&["one", "two"]), page(&["three"]), page(&[])]);
        let pdf = text(&bytes);
        let table = pdf.split("xref\n").nth(1).unwrap();
        let entries: Vec<&str> = table
            .lines()
            .skip(2)
            .take_while(|line| line.ends_with(" n "))
            .collect();
        assert_eq!(entries.len(), 3 + 2 * 3);
        for (index, entry) in entries.iter().enumerate() {
            let offset: usize = entry[..10].parse().unwrap();
            assert!(
                pdf[offset..].starts_with(&format!("{} 0 obj", index + 1)),
                "object {}",
                index + 1
            );
        }
        let start: usize = pdf
            .split("startxref\n")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(pdf[start..].starts_with("xref"));
    }

    #[test]
    fn stream_lengths_are_exact() {
        let pdf = text(&simple_pdf(&[page(&["a line", "another"])]));
        let after = pdf.split("/Length ").nth(1).unwrap();
        let declared: usize = after.split(' ').next().unwrap().parse().unwrap();
        let body = after
            .split(">>\nstream\n")
            .nth(1)
            .unwrap()
            .split("endstream")
            .next()
            .unwrap();
        assert_eq!(declared, body.len());
    }

    #[test]
    fn one_page_object_per_page_and_an_empty_list_still_gives_a_page() {
        assert!(text(&simple_pdf(&[page(&["a"]), page(&["b"]), page(&["c"])])).contains("/Count 3"));
        assert!(text(&simple_pdf(&[])).contains("/Count 1"));
    }

    #[test]
    fn special_characters_are_escaped_and_odd_ones_replaced() {
        let pdf = text(&simple_pdf(&[page(&["Acme (Pty) Ltd \\ \u{e9}\u{1F642}\ttab"])]));
        assert!(pdf.contains(r"(Acme \(Pty\) Ltd \\ ???tab) Tj"), "{pdf}");
    }

    #[test]
    fn the_same_input_always_gives_the_same_bytes() {
        let pages = [page(&["x", "y"]), page(&["z"])];
        assert_eq!(simple_pdf(&pages), simple_pdf(&pages));
    }

    #[test]
    fn lines_beyond_the_page_are_dropped_not_overflowed() {
        let many: Vec<String> = (0..LINES_PER_PAGE + 10).map(|n| format!("line {n}")).collect();
        let pdf = text(&simple_pdf(&[many]));
        assert!(pdf.contains(&format!("(line {}) Tj", LINES_PER_PAGE - 1)));
        assert!(!pdf.contains(&format!("(line {}) Tj", LINES_PER_PAGE)));
    }
}
