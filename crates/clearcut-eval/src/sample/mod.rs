// Written by Kone Tshivhinda & Claude
//! The synthetic sample gold set (ADR-0010).
//!
//! Fifteen made-up invoices with made-up suppliers, written as real PDFs with
//! a text layer and as labels, deterministically: the same code always gives
//! the same files. It is checked into the repository (`crates/clearcut-eval/sample`)
//! so that CI has a small gold set to run the tools against, and a test makes
//! sure the checked-in files are exactly what this code produces.
//!
//! The mix is chosen to exercise the rules, not to look typical: every tax
//! category, a multi-page document, a credit note, a foreign currency, a
//! rounding case, and planted mistakes (a wrong printed VAT, a wrong line
//! total, a due date before the invoice date, a missing supplier address) with
//! the exact findings the validator should report for each.
//!
//! Everything here is invented. The VAT numbers follow the real shape
//! (10 digits starting with 4) but are not issued to anyone.

mod pdf;
pub mod results;

use crate::decimal::{format_money, format_quantity};
use crate::hash::sha256_hex;
use crate::label::{
    Cited, CreditNoteLabel, DocumentRef, DueLabel, Expect, Label, LineLabel, PartyLabel, Provenance, Split,
    Tier, TotalsLabel, Truth, FORMAT,
};
use clearcut_core::{Currency, Invoice, LineItem, Money, TaxCategory};
use pdf::{simple_pdf, LINES_PER_PAGE};
use std::fs;
use std::io;
use std::num::NonZeroU32;
use std::path::Path;
use time::{Date, Duration, Month};

/// One generated case: its label, the PDF, and the text printed on each page
/// (kept so tests can check that a label's page citations are true).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleCase {
    pub label: Label,
    pub document: Vec<u8>,
    pub pages: Vec<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SampleError {
    #[error("the folder is not empty; the sample set is only written into an empty or new folder")]
    NotEmpty,
    #[error("the sample set could not be written ({0})")]
    Io(io::ErrorKind),
}

impl From<io::Error> for SampleError {
    fn from(error: io::Error) -> Self {
        SampleError::Io(error.kind())
    }
}

/// Writes the sample set into `dir`, which must not exist or must be empty
/// (it never overwrites anything).
pub fn write_sample(dir: &Path) -> Result<(), SampleError> {
    if dir.exists() && fs::read_dir(dir)?.next().is_some() {
        return Err(SampleError::NotEmpty);
    }
    for case in cases() {
        let folder = dir.join(&case.label.case);
        fs::create_dir_all(&folder)?;
        let mut text = serde_json::to_string_pretty(&case.label)
            .map_err(|_| SampleError::Io(io::ErrorKind::InvalidData))?;
        text.push('\n');
        fs::write(folder.join("label.json"), text)?;
        fs::write(folder.join("document.pdf"), &case.document)?;
    }
    Ok(())
}

/// All fifteen cases, in order.
pub fn cases() -> Vec<SampleCase> {
    specs().iter().map(build).collect()
}

// ---- the cast ----

struct Org {
    name: &'static str,
    address: &'static str,
    vat: Option<&'static str>,
}

const KAROO: Org = Org {
    name: "Karoo Stationery (Pty) Ltd",
    address: "14 Church Street, Graaff-Reinet, 6280",
    vat: Some("4999900101"),
};
const TAFELBERG: Org = Org {
    name: "Tafelberg Print Works CC",
    address: "7 Loop Street, Cape Town, 8001",
    vat: Some("4999900102"),
};
const HIGHVELD: Org = Org {
    name: "Highveld Export Traders (Pty) Ltd",
    address: "22 Rivonia Road, Sandton, 2196",
    vat: Some("4999900103"),
};
const BERGSIG: Org = Org {
    name: "Bergsig Schools Trust",
    address: "3 Kerk Street, Stellenbosch, 7600",
    vat: Some("4999900104"),
};
const MFUNDO: Org = Org {
    name: "Mfundo Tutoring",
    address: "Unit 2, 9 Smith Street, Durban, 4001",
    vat: None,
};
const INDIGO: Org = Org {
    name: "Indigo Cloud Services Inc",
    address: "100 Market Street, Austin, Texas",
    vat: None,
};
const PROTEA: Org = Org {
    name: "Protea Plumbing (Pty) Ltd",
    address: "18 Voortrekker Road, Bellville, 7530",
    vat: Some("4999900107"),
};
const NAMIB: Org = Org {
    name: "Namib Logistics (Pty) Ltd",
    address: "5 Dock Road, Gqeberha, 6001",
    vat: Some("4999900108"),
};
const DRAKENSBERG: Org = Org {
    name: "Drakensberg Catering CC",
    address: "61 Main Road, Underberg, 3257",
    vat: Some("4999900109"),
};
const FYNBOS: Org = Org {
    name: "Fynbos Software (Pty) Ltd",
    address: "2 Long Street, Cape Town, 8001",
    vat: Some("4999900201"),
};
const LIMPOPO: Org = Org {
    name: "Limpopo Agri Supplies (Pty) Ltd",
    address: "30 Nelson Mandela Drive, Polokwane, 0699",
    vat: Some("4999900202"),
};
const ELAND: Org = Org {
    name: "Eland Office Hire CC",
    address: "12 Kruger Avenue, Pretoria, 0002",
    vat: Some("4999900203"),
};
const KALAHARI: Org = Org {
    name: "Kalahari Print & Pack (Pty) Ltd",
    address: "8 Station Road, Upington, 8801",
    vat: Some("4999900204"),
};
const CLIENT: Org = Org {
    name: "Sample Client (Pty) Ltd",
    address: "1 Example Road, Midrand, 1685",
    vat: Some("4999900301"),
};

// ---- the recipes ----

struct Row {
    description: &'static str,
    quantity_milli: i64,
    unit_price: i64,
    tax: TaxCategory,
}

fn row(description: &'static str, quantity: i64, unit_price: i64, tax: TaxCategory) -> Row {
    Row {
        description,
        quantity_milli: quantity * 1_000,
        unit_price,
        tax,
    }
}

/// A mistake planted in what is printed (the label records the document as
/// printed, so the label holds the mistake and `expect` names the findings).
enum Tweak {
    None,
    /// The printed VAT, and so the printed total, is too high by this much.
    VatTooHigh(i64),
    /// The printed total of this 1-based line is too high by this much.
    LineTotalTooHigh {
        line: usize,
        by: i64,
    },
}

struct Spec {
    id: &'static str,
    split: Split,
    tags: &'static [&'static str],
    title: &'static str,
    supplier: &'static Org,
    /// Print no supplier address (a planted mistake).
    leave_out_address: bool,
    recipient: Option<&'static Org>,
    number: &'static str,
    issued: Date,
    currency: Currency,
    rows: Vec<Row>,
    per_page: usize,
    /// Days after the invoice date the payment falls due, and the printed terms.
    /// Negative days plant a due date before the invoice date.
    due: Option<(i64, &'static str)>,
    credit_note_for: Option<&'static str>,
    tweak: Tweak,
    expect: &'static [&'static str],
}

fn day(year: i32, month: u8, day: u8) -> Date {
    Date::from_calendar_date(year, Month::try_from(month).expect("a month"), day).expect("a real date")
}

fn spec(
    id: &'static str,
    split: Split,
    supplier: &'static Org,
    number: &'static str,
    issued: Date,
    rows: Vec<Row>,
) -> Spec {
    Spec {
        id,
        split,
        tags: &["text-layer", "tax-invoice", "vat-standard"],
        title: "TAX INVOICE",
        supplier,
        leave_out_address: false,
        recipient: None,
        number,
        issued,
        currency: Currency::Zar,
        rows,
        per_page: 30,
        due: None,
        credit_note_for: None,
        tweak: Tweak::None,
        expect: &[],
    }
}

fn specs() -> Vec<Spec> {
    use Split::{Dev, Test};
    use TaxCategory::{Exempt, OutOfScope, Standard, ZeroRated};

    let mut out = Vec::new();

    out.push(spec(
        "g-0001",
        Dev,
        &KAROO,
        "KS-0001",
        day(2026, 2, 10),
        vec![row("A4 paper, box of 5 reams", 3, 18_950, Standard)],
    ));

    let mut s = spec(
        "g-0002",
        Dev,
        &KAROO,
        "KS-0002",
        day(2026, 2, 24),
        vec![
            row("Office chairs", 6, 125_000, Standard),
            row("Standing desks", 2, 480_000, Standard),
            row("Delivery", 1, 35_000, Standard),
        ],
    );
    s.recipient = Some(&CLIENT);
    s.due = Some((30, "30 days"));
    s.tags = &["text-layer", "tax-invoice", "vat-standard", "full-invoice"];
    out.push(s);

    let mut s = spec(
        "g-0003",
        Dev,
        &TAFELBERG,
        "TP-4410",
        day(2026, 3, 3),
        vec![
            row("Business cards, box of 500", 10, 18_500, Standard),
            row("Flyers A5, 1000", 4, 42_000, Standard),
            row("Brochures tri-fold", 5, 65_000, Standard),
            row("Posters A2", 12, 9_500, Standard),
            row("Banners 2m", 3, 78_000, Standard),
            row("Stickers, roll", 8, 14_000, Standard),
            row("Letterheads, ream", 6, 31_000, Standard),
            row("Envelopes DL, box", 9, 12_000, Standard),
            row("Design setup fee", 1, 150_000, Standard),
        ],
    );
    s.recipient = Some(&CLIENT);
    s.per_page = 4;
    s.due = Some((30, "30 days"));
    s.tags = &[
        "text-layer",
        "tax-invoice",
        "vat-standard",
        "multi-page",
        "full-invoice",
    ];
    out.push(s);

    let mut s = spec(
        "g-0004",
        Dev,
        &HIGHVELD,
        "HE-0087",
        day(2026, 3, 12),
        vec![row("Export of machine parts", 40, 85_000, ZeroRated)],
    );
    s.recipient = Some(&CLIENT);
    s.due = Some((60, "60 days"));
    s.tags = &["text-layer", "tax-invoice", "zero-rated", "full-invoice"];
    out.push(s);

    let mut s = spec(
        "g-0005",
        Dev,
        &BERGSIG,
        "BST-221",
        day(2026, 4, 2),
        vec![row("Tuition fees, term 2", 1, 1_250_000, Exempt)],
    );
    s.recipient = Some(&CLIENT);
    s.due = Some((14, "14 days"));
    s.tags = &["text-layer", "invoice", "exempt", "full-invoice"];
    out.push(s);

    let mut s = spec(
        "g-0006",
        Dev,
        &MFUNDO,
        "MT-014",
        day(2026, 4, 15),
        vec![row("Maths tutoring, 1 hour", 3, 15_000, OutOfScope)],
    );
    s.title = "INVOICE";
    s.tags = &["text-layer", "invoice", "no-vat"];
    s.expect = &["compliance:supplier_vat_number"];
    out.push(s);

    let mut s = spec(
        "g-0007",
        Dev,
        &INDIGO,
        "IC-90412",
        day(2026, 3, 31),
        vec![row("Cloud hosting, March", 1, 24_000, OutOfScope)],
    );
    s.title = "INVOICE";
    s.currency = Currency::Usd;
    s.due = Some((15, "Net 15"));
    s.tags = &["text-layer", "invoice", "foreign-currency"];
    out.push(s);

    let mut s = spec(
        "g-0008",
        Dev,
        &PROTEA,
        "PP-2044",
        day(2026, 5, 6),
        vec![row("Refund: faulty valve", 1, 30_000, Standard)],
    );
    s.title = "CREDIT NOTE";
    s.credit_note_for = Some("PP-2031");
    s.tags = &["text-layer", "credit-note", "vat-standard"];
    out.push(s);

    let mut s = spec(
        "g-0009",
        Dev,
        &NAMIB,
        "NL-5530",
        day(2026, 5, 19),
        vec![
            row("Pallet transfer, Gqeberha to Durban", 4, 52_500, Standard),
            row("Fuel surcharge", 1, 18_000, Standard),
        ],
    );
    s.tweak = Tweak::VatTooHigh(10_000);
    s.tags = &["text-layer", "tax-invoice", "vat-standard", "wrong-totals"];
    s.expect = &["totals_mismatch:gross", "totals_mismatch:vat"];
    out.push(s);

    let mut s = spec(
        "g-0010",
        Dev,
        &DRAKENSBERG,
        "DC-0319",
        day(2026, 6, 1),
        vec![
            row("Lunch buffet, per person", 25, 8_500, Standard),
            row("Venue set-up", 1, 25_000, Standard),
        ],
    );
    s.tweak = Tweak::LineTotalTooHigh { line: 2, by: 1_000 };
    s.tags = &["text-layer", "tax-invoice", "vat-standard", "wrong-totals"];
    s.expect = &["line_total_mismatch:2"];
    out.push(s);

    let mut s = spec(
        "g-0011",
        Test,
        &FYNBOS,
        "FS-1180",
        day(2026, 6, 10),
        vec![
            row("Annual licence, 10 seats", 10, 62_000, Standard),
            row("Onboarding session", 2, 95_000, Standard),
        ],
    );
    s.recipient = Some(&CLIENT);
    s.due = Some((30, "30 days"));
    s.tags = &["text-layer", "tax-invoice", "vat-standard", "full-invoice"];
    out.push(s);

    let mut s = spec(
        "g-0012",
        Test,
        &LIMPOPO,
        "LA-7702",
        day(2026, 6, 18),
        vec![
            row("Maize meal, 12.5 kg bag", 120, 9_800, Standard),
            row("Sunflower oil, 5 l", 60, 14_500, Standard),
            row("Sugar, 10 kg bag", 80, 21_000, Standard),
            row("Salt, 1 kg", 200, 1_200, Standard),
            row("Flour, 12.5 kg bag", 50, 17_500, Standard),
            row("Delivery", 1, 45_000, Standard),
        ],
    );
    s.recipient = Some(&CLIENT);
    s.per_page = 4;
    s.due = Some((30, "30 days"));
    s.tags = &[
        "text-layer",
        "tax-invoice",
        "vat-standard",
        "multi-page",
        "full-invoice",
    ];
    out.push(s);

    let mut s = spec(
        "g-0013",
        Test,
        &ELAND,
        "EO-0456",
        day(2026, 7, 8),
        vec![row("Marquee hire, weekend", 1, 185_000, Standard)],
    );
    s.due = Some((-5, "Payable on receipt"));
    s.tags = &["text-layer", "tax-invoice", "vat-standard", "wrong-dates"];
    s.expect = &["due_before_issue"];
    out.push(s);

    let mut s = spec(
        "g-0014",
        Test,
        &KALAHARI,
        "KP-2210",
        day(2026, 7, 22),
        vec![row("Pallet wrap, carton of 6", 12, 28_000, Standard)],
    );
    s.leave_out_address = true;
    s.tags = &["text-layer", "tax-invoice", "vat-standard", "missing-field"];
    s.expect = &["compliance:supplier_address"];
    out.push(s);

    let mut s = spec(
        "g-0015",
        Test,
        &FYNBOS,
        "FS-1204",
        day(2026, 8, 5),
        // 2.5 x 99.99 = 249.975, which rounds half away from zero to 249.98.
        vec![Row {
            description: "Licence seats, per seat",
            quantity_milli: 2_500,
            unit_price: 9_999,
            tax: Standard,
        }],
    );
    s.tags = &["text-layer", "tax-invoice", "vat-standard", "rounding"];
    out.push(s);

    out
}

// ---- turning a recipe into a label and a document ----

fn page_number(n: usize) -> NonZeroU32 {
    NonZeroU32::new(u32::try_from(n).expect("a small page number")).expect("pages start at 1")
}

fn tax_text(tax: TaxCategory) -> &'static str {
    match tax {
        TaxCategory::Standard => "VAT 15%",
        TaxCategory::ZeroRated => "VAT 0%",
        TaxCategory::Exempt => "Exempt",
        TaxCategory::OutOfScope => "No VAT",
    }
}

fn build(spec: &Spec) -> SampleCase {
    let currency = spec.currency;
    let money = |minor| Money::new(minor, currency);

    // What the document says. Line totals and totals come from the same code
    // ClearCut uses to check them, so a document is right unless a tweak makes
    // it wrong.
    let mut totals = Invoice::new(
        "",
        currency,
        spec.rows
            .iter()
            .map(|r| LineItem {
                description: r.description.to_string(),
                quantity_milli: r.quantity_milli,
                unit_price: money(r.unit_price),
                tax: r.tax,
            })
            .collect(),
    )
    .totals()
    .expect("sample amounts are small");
    let mut line_totals: Vec<i64> = spec
        .rows
        .iter()
        .map(|r| {
            money(r.unit_price)
                .mul_milli(r.quantity_milli)
                .expect("sample amounts are small")
                .minor
        })
        .collect();
    match spec.tweak {
        Tweak::None => {}
        Tweak::VatTooHigh(by) => {
            totals.vat = money(totals.vat.minor + by);
            totals.gross = money(totals.gross.minor + by);
        }
        Tweak::LineTotalTooHigh { line, by } => line_totals[line - 1] += by,
    }

    // Layout: header and the first lines on page 1, the rest on later pages,
    // totals and payment details on the last page.
    let page_count = spec.rows.len().div_ceil(spec.per_page).max(1);
    let line_page = |index: usize| index / spec.per_page + 1;
    let mut pages: Vec<Vec<String>> = vec![Vec::new(); page_count];

    let p1 = &mut pages[0];
    p1.push(spec.title.to_string());
    p1.push(spec.supplier.name.to_string());
    if !spec.leave_out_address {
        p1.push(spec.supplier.address.to_string());
    }
    if let Some(vat) = spec.supplier.vat {
        p1.push(format!("VAT No: {vat}"));
    }
    p1.push(format!("Number: {}", spec.number));
    p1.push(format!("Date: {}", spec.issued));
    if let Some(client) = spec.recipient {
        p1.push(format!("Bill to: {}", client.name));
        p1.push(client.address.to_string());
        if let Some(vat) = client.vat {
            p1.push(format!("Customer VAT No: {vat}"));
        }
    }
    p1.push(String::new());
    for (index, r) in spec.rows.iter().enumerate() {
        let page = line_page(index) - 1;
        if page > 0 && index % spec.per_page == 0 {
            pages[page].push(format!("{} (continued)", spec.title));
        }
        pages[page].push(format!(
            "{} | qty {} | price {} | total {} | {}",
            r.description,
            format_quantity(r.quantity_milli),
            format_money(r.unit_price),
            format_money(line_totals[index]),
            tax_text(r.tax)
        ));
    }
    let last = &mut pages[page_count - 1];
    last.push(String::new());
    last.push(format!("Net: {}", format_money(totals.net.minor)));
    last.push(format!("VAT: {}", format_money(totals.vat.minor)));
    last.push(format!(
        "Total: {} {}",
        currency.code(),
        format_money(totals.gross.minor)
    ));
    let due = spec
        .due
        .map(|(days, terms)| (spec.issued + Duration::days(days), terms));
    if let Some((date, terms)) = due {
        last.push(format!("Due date: {date}"));
        last.push(format!("Terms: {terms}"));
    }
    if let Some(original) = spec.credit_note_for {
        last.push(format!(
            "Credit against invoice {original}, amount {}",
            format_money(totals.gross.minor)
        ));
    }
    assert!(
        pages.iter().all(|p| p.len() <= LINES_PER_PAGE),
        "sample {} overflows a page",
        spec.id
    );

    // The label: the same facts, as a person would write them.
    let last_page = page_number(page_count);
    let party = |org: &Org, address: bool| PartyLabel {
        name: org.name.to_string(),
        address: address.then(|| org.address.to_string()),
        vat_number: org.vat.map(str::to_string),
    };
    let truth = Truth {
        currency,
        invoice_number: Some(Cited {
            value: spec.number.to_string(),
            page: page_number(1),
        }),
        invoice_date: Some(Cited {
            value: spec.issued,
            page: page_number(1),
        }),
        supplier: Some(Cited {
            value: party(spec.supplier, !spec.leave_out_address),
            page: page_number(1),
        }),
        recipient: spec.recipient.map(|client| Cited {
            value: party(client, true),
            page: page_number(1),
        }),
        lines: spec
            .rows
            .iter()
            .enumerate()
            .map(|(index, r)| LineLabel {
                page: page_number(line_page(index)),
                description: r.description.to_string(),
                quantity: format_quantity(r.quantity_milli),
                unit_price: format_money(r.unit_price),
                line_total: format_money(line_totals[index]),
                tax: r.tax,
            })
            .collect(),
        stated_totals: Some(TotalsLabel {
            net: format_money(totals.net.minor),
            vat: format_money(totals.vat.minor),
            gross: format_money(totals.gross.minor),
            page: last_page,
        }),
        due: due.map(|(date, terms)| DueLabel {
            date,
            terms: Some(terms.to_string()),
            page: last_page,
        }),
        credit_note: spec.credit_note_for.map(|original| CreditNoteLabel {
            original_invoice_number: original.to_string(),
            amount: format_money(totals.gross.minor),
            page: last_page,
        }),
    };

    let document = simple_pdf(&pages);
    let label = Label {
        format: FORMAT,
        case: spec.id.to_string(),
        document: DocumentRef {
            file: "document.pdf".to_string(),
            sha256: sha256_hex(&document),
            pages: u32::try_from(page_count).expect("a small page count"),
        },
        tags: spec.tags.iter().map(|tag| (*tag).to_string()).collect(),
        split: spec.split,
        provenance: Provenance {
            tier: Tier::Synthetic,
            consent_ref: None,
            anonymised: true,
            retain_until: None,
            labelled_by: "sample-generator".to_string(),
            reviewed_by: None,
        },
        truth,
        expect: Expect {
            findings: spec.expect.iter().map(|key| (*key).to_string()).collect(),
        },
    };
    SampleCase {
        label,
        document,
        pages,
    }
}

#[cfg(test)]
mod tests;
