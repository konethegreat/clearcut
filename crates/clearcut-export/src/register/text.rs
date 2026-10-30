// Written by Kone Tshivhinda & Claude
//! The register in JSON, HTML, Markdown and plain text.
//!
//! Names, descriptions and numbers that came off a document are data: HTML escapes every
//! character that matters, and Markdown cannot take a `|` out of a table cell.

use crate::text::{esc, STYLE};
use clearcut_core::export::Amount;
use clearcut_core::narration::format::{rate_bps, tax_category};
use clearcut_core::register::Register;
use time::Date;

/// The canonical register file: every field, with exact minor units beside each amount.
pub fn json(register: &Register) -> String {
    serde_json::to_string_pretty(register).unwrap_or_default()
}

fn date_text(date: Option<Date>) -> String {
    date.map(|date| date.to_string())
        .unwrap_or_else(|| "no date".to_string())
}

fn amount_text(amount: Option<&Amount>) -> &str {
    amount.map(|amount| amount.text.as_str()).unwrap_or("")
}

fn page_text(page: Option<u32>) -> String {
    page.map(|page| page.to_string()).unwrap_or_default()
}

/// A self-contained page: no script, no outside file.
pub fn html(register: &Register) -> String {
    let mut out = String::new();
    out.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    out.push_str(&format!(
        "<title>Register {}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n",
        esc(&register.selection.label())
    ));
    out.push_str("<h1>Register</h1>\n");
    out.push_str(&format!(
        "<p class=\"muted\">{} &middot; {} document{} &middot; made by ClearCut {}</p>\n",
        esc(&register.selection.label()),
        register.manifest.documents,
        if register.manifest.documents == 1 { "" } else { "s" },
        esc(&register.version)
    ));
    if let Some(rate) = register.manifest.automation_rate_bps {
        out.push_str(&format!(
            "<p class=\"muted\">{} of finished documents needed nobody ({} of {} finished).</p>\n",
            rate_bps(rate),
            register.manifest.done,
            register.manifest.finished
        ));
    }
    if register.manifest.undated_in_tenant > 0 {
        out.push_str(&format!(
            "<p class=\"muted\">Documents with no invoice date in this account: {}.</p>\n",
            register.manifest.undated_in_tenant
        ));
    }

    out.push_str("<h2>Documents</h2>\n<table>\n<thead><tr>");
    for head in [
        "Date", "Supplier", "Number", "Kind", "Status", "Currency", "Net", "VAT", "Gross", "Lines",
    ] {
        out.push_str(&format!("<th>{head}</th>"));
    }
    out.push_str("</tr></thead>\n<tbody>\n");
    for row in &register.rows {
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td></tr>\n",
            esc(&date_text(row.invoice_date)),
            esc(row.supplier.as_deref().unwrap_or("")),
            esc(row.invoice_number.as_deref().unwrap_or("")),
            esc(row.kind.label()),
            esc(row.status.as_str()),
            esc(row.currency.as_deref().unwrap_or("")),
            esc(amount_text(row.net.as_ref())),
            esc(amount_text(row.vat.as_ref())),
            esc(amount_text(row.gross.as_ref())),
            row.line_count
        ));
    }
    out.push_str("</tbody>\n</table>\n");

    // A page cannot usefully carry fifty thousand lines: the workbook and the bundle do.
    if !register.lines.is_empty() {
        out.push_str(&format!(
            "<p class=\"muted\">This page carries the register, the totals and what needs a \
             person. The {} line items are in the workbook and the CSV bundle.</p>\n",
            register.lines.len()
        ));
    }

    if !register.vat_by_category.is_empty() {
        out.push_str("<h2>VAT</h2>\n<table>\n<thead><tr>");
        for head in [
            "Month",
            "Currency",
            "Kind",
            "Category",
            "Rate",
            "Side",
            "Documents",
            "Taxable",
            "VAT",
        ] {
            out.push_str(&format!("<th>{head}</th>"));
        }
        out.push_str("</tr></thead>\n<tbody>\n");
        for group in &register.vat_by_category {
            for (name, part) in [
                ("Ready", group.ready.as_ref()),
                ("Awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                out.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td>\
                     <td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
                     <td class=\"num\">{}</td></tr>\n",
                    esc(&group.month),
                    esc(&group.currency),
                    esc(group.kind.label()),
                    esc(tax_category(group.category)),
                    esc(&rate_bps(group.rate_bps)),
                    name,
                    part.documents,
                    esc(&part.taxable.text),
                    esc(&part.vat.text)
                ));
            }
        }
        out.push_str("</tbody>\n</table>\n");
    }

    if !register.stated_totals.is_empty() {
        out.push_str("<h2>Stated totals</h2>\n<table>\n<thead><tr>");
        for head in [
            "Month",
            "Currency",
            "Kind",
            "Side",
            "Documents",
            "Net",
            "VAT",
            "Gross",
        ] {
            out.push_str(&format!("<th>{head}</th>"));
        }
        out.push_str("</tr></thead>\n<tbody>\n");
        for group in &register.stated_totals {
            for (name, part) in [
                ("Ready", group.ready.as_ref()),
                ("Awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                out.push_str(&format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td>\
                     <td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>\n",
                    esc(&group.month),
                    esc(&group.currency),
                    esc(group.kind.label()),
                    name,
                    part.documents,
                    esc(&part.net.text),
                    esc(&part.vat.text),
                    esc(&part.gross.text)
                ));
            }
        }
        out.push_str("</tbody>\n</table>\n");
    }

    if !register.reconciliation.is_empty() {
        out.push_str("<h2>Reconciliation</h2>\n<table>\n<thead><tr>");
        for head in [
            "Month",
            "Currency",
            "Kind",
            "Calculated VAT",
            "Stated VAT",
            "Difference",
            "Documents that disagree",
        ] {
            out.push_str(&format!("<th>{head}</th>"));
        }
        out.push_str("</tr></thead>\n<tbody>\n");
        for row in &register.reconciliation {
            out.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td>\
                 <td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>\n",
                esc(&row.month),
                esc(&row.currency),
                esc(row.kind.label()),
                esc(&row.calculated_vat.text),
                esc(&row.stated_vat.text),
                esc(&row.difference.text),
                row.documents_disagree
            ));
        }
        out.push_str("</tbody>\n</table>\n");
    }

    out.push_str("<h2>To check</h2>\n");
    if register.to_check.is_empty() {
        out.push_str("<p class=\"muted\">Nothing is waiting for a person.</p>\n");
    } else {
        out.push_str("<table>\n<thead><tr>");
        for head in ["Document", "Reason", "Page", "What to check"] {
            out.push_str(&format!("<th>{head}</th>"));
        }
        out.push_str("</tr></thead>\n<tbody>\n");
        for row in &register.to_check {
            out.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td class=\"num\">{}</td><td>{}</td></tr>\n",
                esc(&row.filename),
                esc(&row.reason),
                esc(&page_text(row.page)),
                esc(&row.sentence)
            ));
        }
        out.push_str("</tbody>\n</table>\n");
    }
    out.push_str(&format!("<p class=\"notice\">{}</p>\n", esc(register.notice)));
    out.push_str("</body>\n</html>\n");
    out
}

fn md(text: &str) -> String {
    text.replace('|', "\\|")
}

/// The register as Markdown, for a note, a wiki or a git repository.
pub fn markdown(register: &Register) -> String {
    let mut out = String::from("# Register\n\n");
    out.push_str(&format!(
        "_{} · {} documents · made by ClearCut {}_\n\n",
        md(&register.selection.label()),
        register.manifest.documents,
        md(&register.version)
    ));
    if let Some(rate) = register.manifest.automation_rate_bps {
        out.push_str(&format!(
            "{} of finished documents needed nobody ({} of {} finished).\n\n",
            rate_bps(rate),
            register.manifest.done,
            register.manifest.finished
        ));
    }

    out.push_str("## Documents\n\n| Date | Supplier | Number | Kind | Status | Currency | Net | VAT | Gross | Lines |\n");
    out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for row in &register.rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            md(&date_text(row.invoice_date)),
            md(row.supplier.as_deref().unwrap_or("")),
            md(row.invoice_number.as_deref().unwrap_or("")),
            row.kind.label(),
            row.status.as_str(),
            md(row.currency.as_deref().unwrap_or("")),
            md(amount_text(row.net.as_ref())),
            md(amount_text(row.vat.as_ref())),
            md(amount_text(row.gross.as_ref())),
            row.line_count
        ));
    }
    out.push('\n');

    if !register.vat_by_category.is_empty() {
        out.push_str(
            "## VAT\n\n| Month | Currency | Kind | Category | Rate | Side | Documents | Taxable | VAT |\n",
        );
        out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
        for group in &register.vat_by_category {
            for (name, part) in [
                ("Ready", group.ready.as_ref()),
                ("Awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                    md(&group.month),
                    md(&group.currency),
                    group.kind.label(),
                    md(tax_category(group.category)),
                    md(&rate_bps(group.rate_bps)),
                    name,
                    part.documents,
                    md(&part.taxable.text),
                    md(&part.vat.text)
                ));
            }
        }
        out.push('\n');
    }

    if !register.reconciliation.is_empty() {
        out.push_str("## Reconciliation\n\n| Month | Currency | Kind | Calculated VAT | Stated VAT | Difference | Documents that disagree |\n");
        out.push_str("| --- | --- | --- | --- | --- | --- | --- |\n");
        for row in &register.reconciliation {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                md(&row.month),
                md(&row.currency),
                row.kind.label(),
                md(&row.calculated_vat.text),
                md(&row.stated_vat.text),
                md(&row.difference.text),
                row.documents_disagree
            ));
        }
        out.push('\n');
    }

    out.push_str("## To check\n\n");
    if register.to_check.is_empty() {
        out.push_str("Nothing is waiting for a person.\n\n");
    } else {
        out.push_str("| Document | Reason | Page | What to check |\n| --- | --- | --- | --- |\n");
        for row in &register.to_check {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                md(&row.filename),
                md(&row.reason),
                md(&page_text(row.page)),
                md(&row.sentence)
            ));
        }
        out.push('\n');
    }
    out.push_str(&format!("---\n\n{}\n", register.notice));
    out
}

/// The register as plain text, for a terminal, an email or a quick check.
pub fn plain(register: &Register) -> String {
    let mut out = String::from("REGISTER\n");
    out.push_str(&format!("{}\n", register.selection.label()));
    out.push_str(&format!(
        "{} documents, {} done, {} awaiting review, {} still reading\n",
        register.manifest.documents,
        register.manifest.done,
        register.manifest.needs_review,
        register.manifest.still_reading
    ));
    if let Some(rate) = register.manifest.automation_rate_bps {
        out.push_str(&format!(
            "{} of finished documents needed nobody\n",
            rate_bps(rate)
        ));
    }

    out.push_str("\nDOCUMENTS\n");
    for row in &register.rows {
        out.push_str(&format!(
            "{}  {}  {}  {}  {}  {}  net {}  VAT {}  gross {}  {} lines\n",
            date_text(row.invoice_date),
            row.supplier.as_deref().unwrap_or(""),
            row.invoice_number.as_deref().unwrap_or(""),
            row.kind.label(),
            row.status.as_str(),
            row.currency.as_deref().unwrap_or(""),
            amount_text(row.net.as_ref()),
            amount_text(row.vat.as_ref()),
            amount_text(row.gross.as_ref()),
            row.line_count
        ));
    }

    // A page cannot usefully carry fifty thousand lines: the workbook and the bundle do.
    if !register.lines.is_empty() {
        out.push_str(&format!(
            "\n{} line items are in the workbook and the CSV bundle.\n",
            register.lines.len()
        ));
    }

    if !register.vat_by_category.is_empty() {
        out.push_str("\nVAT\n");
        for group in &register.vat_by_category {
            for (name, part) in [
                ("ready", group.ready.as_ref()),
                ("awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                out.push_str(&format!(
                    "{}  {}  {}  {}  {}  {}  {} doc(s)  {} taxable  {} VAT\n",
                    group.month,
                    group.currency,
                    group.kind.label(),
                    tax_category(group.category),
                    rate_bps(group.rate_bps),
                    name,
                    part.documents,
                    part.taxable.text,
                    part.vat.text
                ));
            }
        }
    }

    if !register.stated_totals.is_empty() {
        out.push_str("\nSTATED TOTALS\n");
        for group in &register.stated_totals {
            for (name, part) in [
                ("ready", group.ready.as_ref()),
                ("awaiting review", group.awaiting.as_ref()),
            ] {
                let Some(part) = part else {
                    continue;
                };
                out.push_str(&format!(
                    "{}  {}  {}  {}  {} doc(s)  net {}  VAT {}  gross {}\n",
                    group.month,
                    group.currency,
                    group.kind.label(),
                    name,
                    part.documents,
                    part.net.text,
                    part.vat.text,
                    part.gross.text
                ));
            }
        }
    }

    if !register.reconciliation.is_empty() {
        out.push_str("\nRECONCILIATION\n");
        for row in &register.reconciliation {
            out.push_str(&format!(
                "{}  {}  {}  calculated {}  stated {}  difference {}  {} disagree\n",
                row.month,
                row.currency,
                row.kind.label(),
                row.calculated_vat.text,
                row.stated_vat.text,
                row.difference.text,
                row.documents_disagree
            ));
        }
    }

    out.push_str("\nTO CHECK\n");
    if register.to_check.is_empty() {
        out.push_str("Nothing is waiting for a person.\n");
    }
    for row in &register.to_check {
        let page = row.page.map(|page| format!("  page {page}")).unwrap_or_default();
        out.push_str(&format!(
            "{}  {}  {}{}  {}\n",
            row.filename, row.reason, row.document_id, page, row.sentence
        ));
    }

    out.push_str(&format!("\n{}\n", register.notice));
    out
}
