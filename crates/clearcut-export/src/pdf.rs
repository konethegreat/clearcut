// Written by Kone Tshivhinda & Claude
//! The PDF report: A4, plain Helvetica, tables, a footer with the notice and page numbers.
//!
//! The standard Helvetica font is built into every PDF reader, so nothing is embedded and the
//! file stays small. Its character widths are known, which lets lines wrap and amounts right-align
//! exactly. Characters Helvetica's Windows encoding does not have are written as `?` rather than
//! as something wrong.

use clearcut_core::export::ExportModel;
use printpdf::{
    BuiltinFont, Color, Line, LinePoint, Mm, Op, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Point,
    Pt, Rgb, TextItem,
};

const PAGE_W: f32 = 595.276;
const PAGE_H: f32 = 841.89;
const MARGIN: f32 = 51.0;
const CONTENT_W: f32 = PAGE_W - 2.0 * MARGIN;
const BOTTOM: f32 = MARGIN + 26.0;
const TOP: f32 = PAGE_H - MARGIN;

#[derive(Debug)]
pub struct PdfError;

/// Helvetica advance widths for ASCII 32 to 126, in thousandths of the font size.
const WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556,
    556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278,
    500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469,
    556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500,
    278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

fn char_width(c: char, bold: bool) -> f32 {
    let base = match c as u32 {
        32..=126 => f32::from(WIDTHS[(c as u32 - 32) as usize]),
        _ => 556.0,
    };
    if bold {
        base * 1.06
    } else {
        base
    }
}

fn width(text: &str, size: f32, bold: bool) -> f32 {
    text.chars().map(|c| char_width(c, bold)).sum::<f32>() * size / 1000.0
}

/// Text Helvetica's Windows encoding can show: Latin-1 as it is, no-break space as a space, the rest `?`.
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{a0}' => ' ',
            '\n' | '\r' | '\t' => ' ',
            c if (c as u32) < 32 => ' ',
            c if (c as u32) <= 0xFF => c,
            '\u{2022}' | '\u{b7}' => '\u{b7}',
            '\u{2013}' | '\u{2014}' => '-',
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            _ => '?',
        })
        .collect()
}

/// Breaks `text` into lines no wider than `max` points.
fn wrap(text: &str, size: f32, bold: bool, max: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in clean(text).split(' ').filter(|w| !w.is_empty()) {
        // A word wider than the line is cut where it must be.
        let mut rest = word.to_string();
        while width(&rest, size, bold) > max {
            let mut cut = 0;
            for (i, c) in rest.char_indices() {
                if width(&rest[..i + c.len_utf8()], size, bold) > max {
                    break;
                }
                cut = i + c.len_utf8();
            }
            let cut = cut.max(rest.chars().next().map_or(1, char::len_utf8));
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            lines.push(rest[..cut].to_string());
            rest = rest[cut..].to_string();
        }
        let candidate = if line.is_empty() {
            rest.clone()
        } else {
            format!("{line} {rest}")
        };
        if width(&candidate, size, bold) > max && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            line = rest;
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

struct Layout {
    pages: Vec<Vec<Op>>,
    y: f32,
}

#[derive(Clone, Copy)]
struct Style {
    size: f32,
    bold: bool,
    grey: bool,
}

const BODY: Style = Style {
    size: 10.0,
    bold: false,
    grey: false,
};

impl Layout {
    fn new() -> Layout {
        Layout {
            pages: vec![Vec::new()],
            y: TOP,
        }
    }

    fn page(&mut self) -> &mut Vec<Op> {
        self.pages.last_mut().expect("there is always a page")
    }

    fn break_page(&mut self) {
        self.pages.push(Vec::new());
        self.y = TOP;
    }

    /// Makes room for `height`, starting a new page if it does not fit.
    fn room(&mut self, height: f32) {
        if self.y - height < BOTTOM {
            self.break_page();
        }
    }

    fn text_at(ops: &mut Vec<Op>, x: f32, y: f32, style: Style, text: &str) {
        let colour = if style.grey {
            Rgb::new(0.37, 0.42, 0.45, None)
        } else {
            Rgb::new(0.11, 0.14, 0.15, None)
        };
        ops.push(Op::StartTextSection);
        ops.push(Op::SetFillColor {
            col: Color::Rgb(colour),
        });
        ops.push(Op::SetTextCursor {
            pos: Point { x: Pt(x), y: Pt(y) },
        });
        ops.push(Op::SetFont {
            font: PdfFontHandle::Builtin(if style.bold {
                BuiltinFont::HelveticaBold
            } else {
                BuiltinFont::Helvetica
            }),
            size: Pt(style.size),
        });
        ops.push(Op::ShowText {
            items: vec![TextItem::Text(text.to_string())],
        });
        ops.push(Op::EndTextSection);
    }

    fn rule(ops: &mut Vec<Op>, y: f32) {
        ops.push(Op::SetOutlineColor {
            col: Color::Rgb(Rgb::new(0.8, 0.78, 0.75, None)),
        });
        ops.push(Op::SetOutlineThickness { pt: Pt(0.5) });
        ops.push(Op::DrawLine {
            line: Line {
                points: vec![
                    LinePoint {
                        p: Point {
                            x: Pt(MARGIN),
                            y: Pt(y),
                        },
                        bezier: false,
                    },
                    LinePoint {
                        p: Point {
                            x: Pt(PAGE_W - MARGIN),
                            y: Pt(y),
                        },
                        bezier: false,
                    },
                ],
                is_closed: false,
            },
        });
    }

    /// Wrapped text at the left margin (plus `indent`), a line after another.
    fn paragraph(&mut self, text: &str, style: Style, indent: f32) {
        let lead = style.size * 1.35;
        for line in wrap(text, style.size, style.bold, CONTENT_W - indent) {
            self.room(lead);
            self.y -= lead;
            let y = self.y;
            Layout::text_at(self.page(), MARGIN + indent, y, style, &line);
        }
    }

    fn heading(&mut self, text: &str, size: f32) {
        // A heading is never left alone at the foot of a page: it needs its first lines with it.
        self.room(size * 3.0 + 60.0);
        self.y -= size * 0.6;
        self.paragraph(
            text,
            Style {
                size,
                bold: true,
                grey: false,
            },
            0.0,
        );
        if size >= 12.0 {
            let y = self.y - 3.0;
            Layout::rule(self.page(), y);
            self.y -= 4.0;
        }
        self.y -= 3.0;
    }

    fn bullet(&mut self, text: &str, cite: &str) {
        let lead = BODY.size * 1.35;
        let lines = wrap(text, BODY.size, false, CONTENT_W - 14.0);
        self.room(lead * lines.len().min(2) as f32);
        for (i, line) in lines.iter().enumerate() {
            self.room(lead);
            self.y -= lead;
            let y = self.y;
            if i == 0 {
                Layout::text_at(self.page(), MARGIN + 2.0, y, BODY, "\u{b7}");
            }
            Layout::text_at(self.page(), MARGIN + 14.0, y, BODY, line);
        }
        if !cite.is_empty() {
            let small = Style {
                size: 7.5,
                bold: false,
                grey: true,
            };
            for line in wrap(&format!("[{cite}]"), small.size, false, CONTENT_W - 14.0) {
                self.room(small.size * 1.3);
                self.y -= small.size * 1.3;
                let y = self.y;
                Layout::text_at(self.page(), MARGIN + 14.0, y, small, &line);
            }
        }
        self.y -= 3.0;
    }

    /// A table: a header row, then rows whose cells wrap. `right` columns are right-aligned.
    fn table(&mut self, head: &[&str], widths: &[f32], right: &[bool], rows: &[Vec<String>], size: f32) {
        let lead = size * 1.3;
        let pad = 3.0;
        // Text that follows a right-aligned column starts a little further in, so the two do not run together.
        let left_pad = |i: usize| {
            if i > 0 && right.get(i - 1) == Some(&true) {
                10.0
            } else {
                pad
            }
        };
        let draw_head = |layout: &mut Layout| {
            layout.room(lead + 8.0);
            layout.y -= lead;
            let y = layout.y;
            let mut x = MARGIN;
            for (i, h) in head.iter().enumerate() {
                let style = Style {
                    size,
                    bold: true,
                    grey: true,
                };
                let at = if right.get(i) == Some(&true) {
                    x + widths[i] - pad - width(&clean(h), size, true)
                } else {
                    x + left_pad(i)
                };
                Layout::text_at(layout.page(), at, y, style, &clean(h));
                x += widths[i];
            }
            let line_y = y - 3.0;
            Layout::rule(layout.page(), line_y);
            layout.y -= 4.0;
        };
        draw_head(self);
        for row in rows {
            let cells: Vec<Vec<String>> = row
                .iter()
                .enumerate()
                .map(|(i, c)| wrap(c, size, false, widths[i] - pad - left_pad(i)))
                .collect();
            let lines = cells.iter().map(Vec::len).max().unwrap_or(1);
            let height = lead * lines as f32 + 4.0;
            if self.y - height < BOTTOM {
                self.break_page();
                draw_head(self);
            }
            let top = self.y;
            let mut x = MARGIN;
            for (i, cell) in cells.iter().enumerate() {
                for (n, line) in cell.iter().enumerate() {
                    let y = top - lead * (n as f32 + 1.0);
                    let at = if right.get(i) == Some(&true) {
                        x + widths[i] - pad - width(line, size, false)
                    } else {
                        x + left_pad(i)
                    };
                    Layout::text_at(
                        self.page(),
                        at,
                        y,
                        Style {
                            size,
                            bold: false,
                            grey: false,
                        },
                        line,
                    );
                }
                x += widths[i];
            }
            self.y -= height;
            let line_y = self.y + 1.5;
            Layout::rule(self.page(), line_y);
        }
        self.y -= 6.0;
    }
}

pub fn pdf(model: &ExportModel) -> Result<Vec<u8>, PdfError> {
    let mut layout = Layout::new();

    layout.paragraph(
        &model.document.filename,
        Style {
            size: 18.0,
            bold: true,
            grey: false,
        },
        0.0,
    );
    let facts = format!(
        "Status: {}{}",
        model.document.status,
        model
            .document
            .page_count
            .map(|n| format!(" \u{b7} {n} page{}", if n == 1 { "" } else { "s" }))
            .unwrap_or_default()
    );
    layout.paragraph(
        &facts,
        Style {
            size: 10.0,
            bold: false,
            grey: true,
        },
        0.0,
    );

    if !model.summary.is_empty() {
        layout.heading("Summary", 13.0);
        for s in &model.summary {
            layout.bullet(&s.text, &s.cites.join(", "));
        }
    }
    if !model.sections.is_empty() {
        layout.heading("Report", 13.0);
        for section in &model.sections {
            if let Some(title) = &section.heading {
                layout.heading(title, 10.5);
            }
            for s in &section.sentences {
                layout.bullet(&s.text, &s.cites.join(", "));
            }
        }
    }
    if !model.lines.is_empty() {
        layout.heading("Line items", 13.0);
        let rows: Vec<Vec<String>> = model
            .lines
            .iter()
            .map(|l| {
                vec![
                    l.number.to_string(),
                    l.description.clone(),
                    l.quantity.clone(),
                    l.unit_price.text.clone(),
                    l.line_total.text.clone(),
                    l.tax.clone(),
                ]
            })
            .collect();
        layout.table(
            &["No", "Description", "Qty", "Unit price", "Line total", "Tax"],
            &[26.0, 187.0, 44.0, 76.0, 76.0, 84.0],
            &[true, false, true, true, true, false],
            &rows,
            9.0,
        );
    }
    if !model.totals.is_empty() {
        layout.heading("Totals", 13.0);
        let rows: Vec<Vec<String>> = model
            .totals
            .iter()
            .map(|t| vec![t.label.clone(), t.amount.text.clone(), t.basis.to_string()])
            .collect();
        layout.table(
            &["", "Amount", "Basis"],
            &[220.0, 110.0, 163.0],
            &[false, true, false],
            &rows,
            9.0,
        );
    }
    if !model.checks.is_empty() {
        layout.heading("What the checks found", 13.0);
        let rows: Vec<Vec<String>> = model
            .checks
            .iter()
            .map(|c| vec![c.severity.clone(), c.text.clone(), c.page.to_string()])
            .collect();
        layout.table(
            &["Severity", "Statement", "Page"],
            &[58.0, 403.0, 32.0],
            &[false, false, true],
            &rows,
            9.0,
        );
    }
    if !model.events.is_empty() {
        layout.heading("Everything that was read", 13.0);
        let rows: Vec<Vec<String>> = model
            .events
            .iter()
            .map(|e| {
                vec![
                    format!("{} - {}", e.group, e.kind.replace('_', " ")),
                    e.text.clone(),
                    e.page.to_string(),
                    format!("{}%", e.confidence_per_mille / 10),
                ]
            })
            .collect();
        layout.table(
            &["What", "Statement", "Page", "Sure"],
            &[118.0, 302.0, 32.0, 41.0],
            &[false, false, true, true],
            &rows,
            8.0,
        );
    }
    if !model.sources.is_empty() {
        layout.heading("Where it was read", 13.0);
        let rows: Vec<Vec<String>> = model
            .sources
            .iter()
            .map(|s| {
                vec![
                    s.page.to_string(),
                    s.events.to_string(),
                    format!("{}%", s.lowest_confidence_per_mille / 10),
                    s.read_by.join(", "),
                ]
            })
            .collect();
        layout.table(
            &["Page", "Events", "Least sure", "Read by"],
            &[40.0, 50.0, 70.0, 333.0],
            &[true, true, true, false],
            &rows,
            9.0,
        );
    }

    // The footer goes on once the number of pages is known.
    let total = layout.pages.len();
    let small = Style {
        size: 7.0,
        bold: false,
        grey: true,
    };
    for (i, ops) in layout.pages.iter_mut().enumerate() {
        let notice = wrap(model.notice, small.size, false, CONTENT_W - 60.0);
        for (n, line) in notice.iter().enumerate() {
            Layout::text_at(
                ops,
                MARGIN,
                MARGIN - 4.0 + 9.0 * (notice.len() - 1 - n) as f32,
                small,
                line,
            );
        }
        let label = format!("Page {} of {}", i + 1, total);
        Layout::text_at(
            ops,
            PAGE_W - MARGIN - width(&label, small.size, false),
            MARGIN - 4.0,
            small,
            &label,
        );
    }

    let mut doc = PdfDocument::new(&clean(&model.document.filename));
    let pages: Vec<PdfPage> = layout
        .pages
        .into_iter()
        .map(|ops| PdfPage::new(Mm(210.0), Mm(297.0), ops))
        .collect();
    let mut warnings = Vec::new();
    let bytes = doc
        .with_pages(pages)
        .save(&PdfSaveOptions::default(), &mut warnings);
    if bytes.is_empty() {
        return Err(PdfError);
    }
    Ok(bytes)
}
