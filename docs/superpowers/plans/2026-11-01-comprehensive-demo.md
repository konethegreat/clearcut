# Comprehensive demo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the two-sample demo into a made-up quarter: 18 prepared supplier documents, a library home, a full journey for every document, four review kinds, a workspace, a review queue, a register with five in-page downloads, and a batch replay, all without a key, an API call, a model, an upload or stored state.

**Architecture:** The demo stays inside `crates/clearcut-server/src/web/app.js` as prepared data plus page memory. Every line carries its own prepared net, VAT and gross; the page only groups and adds those values. A module-level `Map` holds confirmations keyed by document id and is read by every screen. The real API, database, worker, OpenAPI and CSS tokens are untouched.

**Tech Stack:** Vanilla JavaScript served by the Rust web server, CSS custom properties already in `app.css`, Rust string-level web tests in `web.rs`, `node --check`, `cargo test -p clearcut-server`.

**Spec:** [docs/superpowers/specs/2026-11-01-comprehensive-demo-design.md](../specs/2026-11-01-comprehensive-demo-design.md). Read it with this plan. Its isolation, honesty and no-key rules bind every task.

## Global Constraints

- The demo contains no call to `api()` or `fetch()`, no key, no `Authorization` header, no tenant, no database, no upload, no model call, no `localStorage`, `sessionStorage` or `indexedDB`, and no stored file. The only network requests are the page, `app.js`, `app.css` and the favicon.
- Money is integer cents. Every line carries prepared `net`, `vat` and `gross`. The page never derives VAT, rounds VAT or infers a tax rule; it groups and adds prepared values.
- Every supplier, person, address and number is fictional. No bank details. The fine comes from "Cape Metro Fines".
- The page keeps one `<script>` tag, no external URL, no inline handler, and never uses `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write`, `eval(` or `new Function`.
- The CSP, the alpha banner, the light-first editorial tokens and the existing `.table-wrap`, `.card`, `.banner`, `.badge` styles stay as they are. No new colours.
- User-facing copy uses no em dashes. Use full stops, commas or semicolons.
- Conventional commits. One branch and one small PR per slice. CI green before squash merge.
- Gate for every slice: `node --check crates/clearcut-server/src/web/app.js`, `cargo fmt --all`, `cargo clippy -p clearcut-server --all-targets -- -D warnings`, `cargo test -p clearcut-server`. CI runs the whole workspace, the contract and the compose build.

**Rulings already made for this plan:**

- **JSON and HTML cannot literally start with the notice.** JSON's first field is `notice`; HTML carries the notice as a comment immediately after `<!doctype html>` and again in the body. CSV, Markdown and plain text start with the notice as their first line. This is the only reading that keeps every file valid.
- **Named screens and their links arrive with their slice.** A slice never ships a link to a screen that does not exist. The final home and workspace match the spec exactly.
- **The demo control that clears state is called "Start over"** and sits in the strip beside "Back to the demo" and "Exit demo". All three clear the confirmation map where they leave the demo.

---

## File Structure

- `crates/clearcut-server/src/web/app.js` — the whole demo: data, helpers, screens, register, downloads, batch, router. No new file, because the page must keep one script tag.
- `crates/clearcut-server/src/web/app.css` — demo layout classes, reusing existing tokens. New rules go immediately before the `/* small */` media block.
- `crates/clearcut-server/src/web.rs` — the Rust tests that pin the data, the routes, the copy and the isolation.
- `docs/LAUNCH.md` — the demo paragraph.
- `docs/WORKLOG.md` — one entry when the demo ships.

### Data shape (used by every task)

```js
{
  id: "pp-0091",
  file: "paper-people-pp-0091.pdf",
  supplier: "Paper People",
  address: "Unit 4, Montague Park, Montague Gardens, 7441",
  supplierVat: "4112233445",
  number: "PP-0091",
  date: "2026-05-14",            // null when the page has no date
  dateText: "14 May 2026",       // "No date" when null
  dueText: "13 June 2026",
  pages: 4,
  kind: "invoice",               // invoice | credit_note | unclassified
  currency: "ZAR",               // ZAR | USD
  status: "needs_review",        // done | needs_review | processing
  held: "line",                  // null | figure | currency | line | date
  note: "Four pages and 45 lines; one line is a cent out.",
  lines: [{ page, description, quantity, unit, net, vat, gross, tax, inclusive }],
  net: 4821649,
  vat: 723247,
  gross: 5544896,
  grossRead: 1250100,            // only BST, the reading's wrong total
  grossPrinted: 1250000,         // only BST, the page's total
  reason: "...",                 // only the four held documents
  check: { label, page, sentence }, // only the four held documents
  checks, confirmedChecks, report, reportConfirmed // only the four held documents
}
```

Helpers produced in Task 1 and used by every later task:

- `demoDoc(id) -> document | null`
- `demoResolve(doc) -> document` (applies a confirmation, sets `status: "done"` and `confirmed: true`)
- `demoHeldResolution(doc) -> object`
- `demoReset() -> void`
- `demoMoney(minor, currency) -> string` (`"R 12 500.00"` or `"USD 2 400.00"`)
- `demoStatedGross(doc) -> number | null` (BST's reading total until confirmed, `null` while processing)
- `demoChecks(doc) -> array`, `demoReport(doc) -> array`
- `demoKind(doc) -> string` (the display word)
- `demoCsvCell(value) -> string`, `demoMdCell(value) -> string`, `demoEscape(value) -> string`

---

## Slice 1: the quarter, the home, the journey and the four reviews

Branch: `feat/demo-quarter`.

### Task 1: Replace the two-sample demo with the prepared quarter

**Files:**
- Modify: `crates/clearcut-server/src/web/app.js:326-681` (exit handler and the whole demo section)
- Modify: `crates/clearcut-server/src/web/app.js:1746-1764` (router)
- Modify: `crates/clearcut-server/src/web/app.css` (before the `/* small */` marker)

**Interfaces:**
- Consumes: `h`, `show`, `stopPolling`, `badge`, `STATUS`, `REPO`, `timer`, `saveFile` (not used by the demo), `main`.
- Produces: `DEMO_DOCUMENTS`, `DEMO_PERIOD`, `DEMO_AUTOMATION_BPS`, `demoDoc`, `demoResolve`, `demoHeldResolution`, `demoReset`, `demoMoney`, `demoStatedGross`, `demoCard`, `demoHome`, `demoJourney`, `demoTimeline`, `demoDocumentScreen`, `demoReview`, `demoReviewSheet`, `demoReviewReading`, `demoConfirmLabel`, `demoConfirmationNote`, `demoChecks`, `demoReport`, `demoExport`, `demoExportPanel`, `demoEndCard`, `demoStrip`, `demoChecksFor`, `demoReportFor`.

- [ ] **Step 1: Write the failing Rust marker test**

In `crates/clearcut-server/src/web.rs`, replace the body of `the_page_offers_a_demo_that_needs_no_key` with:

```rust
    #[tokio::test]
    async fn the_page_offers_a_demo_that_needs_no_key() {
        let (_, _, html) = fetch("/").await;
        assert!(html.contains("href=\"#/demo\""), "the header links the demo");
        let (status, _, js) = fetch("/app.js").await;
        assert_eq!(status, StatusCode::OK);
        for marker in [
            "const DEMO_DOCUMENTS",
            "const DEMO_PERIOD = \"1 March to 31 May 2026\"",
            "Sample documents. Nothing here is saved.",
            "Kloof Street Coffee Roasters",
            "March to May 2026",
            "Eighteen supplier documents",
            "#/demo/invoice/",
            "#/demo/document/",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
    }
```

Add a second test below it:

```rust
    #[tokio::test]
    async fn the_demo_keeps_its_prepared_arithmetic() {
        let (_, _, js) = fetch("/app.js").await;
        for marker in [
            "const DEMO_AUTOMATION_BPS = 7650",
            "net: 4821649",
            "vat: 723247",
            "gross: 5544896",
            "grossRead: 1250100",
            "grossPrinted: 1250000",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p clearcut-server web::tests::the_page_offers_a_demo_that_needs_no_key web::tests::the_demo_keeps_its_prepared_arithmetic`

Expected: FAIL, because `const DEMO_DOCUMENTS` and the new markers are not in `app.js` yet.

- [ ] **Step 3: Replace the exit handler**

Replace `crates/clearcut-server/src/web/app.js:326-328`:

```js
exitDemoButton.addEventListener("click", () => {
  location.hash = "#/";
});
```

with:

```js
exitDemoButton.addEventListener("click", () => {
  stopPolling();
  demoReset();
  location.hash = "#/";
});
```

- [ ] **Step 4: Replace the whole demo section**

Replace everything from `// ---------------------------------------------------------------- 1b. the demo ----` through `function demoEndCard() { ... }` (immediately before `// ---------------------------------------------------------------- bulk intake ----`) with the code below. Do not keep `DEMO_SAMPLES`, `demoSample` or any old demo function.

```js
// ---------------------------------------------------------------- 1b. the demo ----
// Prepared from a made-up roastery's first quarter. Nothing here calls the API, holds a key
// or names a tenant: the demo is data in this file and state in this page, so it cannot see
// or touch an account.

const DEMO_NOTE = "Sample documents. Nothing here is saved.";
const DEMO_PERIOD = "1 March to 31 May 2026";
const DEMO_AUTOMATION_BPS = 7650; // 13 of 17 finished documents never needed a person

const DEMO_KIND = { invoice: "Invoice", credit_note: "Credit note", unclassified: "Unclassified" };
const DEMO_TAX = { standard: "Standard", zero_rated: "Zero-rated", exempt: "Exempt", out_of_scope: "Outside scope" };
const DEMO_RATE = { standard: 1500, zero_rated: 0, exempt: 0, out_of_scope: 0 };
const DEMO_KIND_ORDER = { invoice: 0, credit_note: 1, unclassified: 2 };
const DEMO_TAX_ORDER = { standard: 0, zero_rated: 1, exempt: 2, out_of_scope: 3 };

const demoLine = (page, description, quantity, unit, net, vat, gross, tax, inclusive) =>
  ({ page, description, quantity, unit, net, vat, gross, tax: tax || "standard", inclusive: Boolean(inclusive) });

const DEMO_DOCUMENTS = [
  {
    id: "tp-4410",
    file: "tafelberg-tp-4410.pdf",
    supplier: "Tafelberg Print Works CC",
    address: "12 Bree Street, Cape Town, 8001",
    supplierVat: "4999900102",
    number: "TP-4410",
    date: "2026-03-03",
    dateText: "3 March 2026",
    dueText: "17 March 2026",
    pages: 3,
    kind: "invoice",
    currency: "ZAR",
    status: "done",
    note: "Nine standard lines over three pages, read and checked to the cent.",
    lines: [
      demoLine(1, "Business cards, box of 500", 10, 18500, 185000, 27750, 212750),
      demoLine(1, "Flyers A5, 1000", 4, 42000, 168000, 25200, 193200),
      demoLine(1, "Brochures tri-fold", 5, 65000, 325000, 48750, 373750),
      demoLine(1, "Posters A2", 12, 9500, 114000, 17100, 131100),
      demoLine(2, "Banners 2m", 3, 78000, 234000, 35100, 269100),
      demoLine(2, "Stickers, roll", 8, 14000, 112000, 16800, 128800),
      demoLine(2, "Letterheads, ream", 6, 31000, 186000, 27900, 213900),
      demoLine(2, "Envelopes DL, box", 9, 12000, 108000, 16200, 124200),
      demoLine(3, "Design setup fee", 1, 150000, 150000, 22500, 172500),
    ],
    net: 1582000,
    vat: 237300,
    gross: 1819300,
  },
  {
    id: "crs-8821",
    file: "cape-roasters-crs-8821.pdf",
    supplier: "Cape Roasters Supply",
    address: "44 Marine Drive, Paarden Eiland, 7405",
    supplierVat: "4880123456",
    number: "CRS-8821",
    date: "2026-03-05",
    dateText: "5 March 2026",
    dueText: "19 March 2026",
    pages: 1,
    kind: "invoice",
    currency: "ZAR",
    status: "done",
    note: "Green beans, filters and delivery, with VAT at 15%.",
    lines: [
      demoLine(1, "Green beans, 20 kg", 20, 70000, 1400000, 210000, 1610000),
      demoLine(1, "Filters, case of 12", 1, 420000, 420000, 63000, 483000),
      demoLine(1, "Delivery", 1, 120000, 120000, 18000, 138000),
    ],
    net: 1940000,
    vat: 291000,
    gross: 2231000,
  },
  {
    id: "dd-1740",
    file: "drakenstein-dd-1740.pdf",
    supplier: "Drakenstein Dairy Co-op",
    address: "R45 Main Road, Wellington, 7655",
    supplierVat: "4012345678",
    number: "DD-1740",
    date: "2026-03-12",
    dateText: "12 March 2026",
    dueText: "26 March 2026",
    pages: 1,
    kind: "invoice",
    currency: "ZAR",
    status: "done",
    note: "Milk is zero-rated and cream is standard, on one invoice.",
    lines: [
      demoLine(1, "Milk, 200 L", 200, 2500, 500000, 0, 500000, "zero_rated"),
      demoLine(1, "Cream, 40 L", 40, 3000, 120000, 18000, 138000),
    ],
    net: 620000,
    vat: 18000,
    gross: 638000,
  },
  {
    id: "mf-8092",
    file: "metro-fines-mf-8092.pdf",
    supplier: "Cape Metro Fines",
    address: "Private Bag X1, Bellville, 7535",
    supplierVat: null,
    number: "MF-8092",
    date: "2026-03-18",
    dateText: "18 March 2026",
    dueText: "1 April 2026",
    pages: 1,
    kind: "unclassified",
    currency: "ZAR",
    status: "done",
    note: "A fine, not a tax invoice. No VAT is claimed.",
    lines: [
      demoLine(1, "Parking fine, 18 March", 1, 75000, 75000, 0, 75000, "out_of_scope"),
    ],
    net: 75000,
    vat: 0,
    gross: 75000,
  },
  {
    id: "wf-55302",
    file: "wifi-fibre-wf-55302.pdf",
    supplier: "Wifi & Fibre",
    address: "9 Long Street, Cape Town, 8001",
    supplierVat: "4660011223",
    number: "WF-55302",
    date: "2026-03-31",
    dateText: "31 March 2026",
    dueText: "30 April 2026",
    pages: 1,
    kind: "invoice",
    currency: "ZAR",
    status: "done",
    note: "One monthly service line.",
    lines: [
      demoLine(1, "Fibre line, March", 1, 89900, 89900, 13485, 103385),
    ],
    net: 89900,
    vat: 13485,
    gross: 103385,
  },
  {
    id: "soc-114",
    file: "somerset-soc-114.pdf",
    supplier: "Somerset Office Cleaners",
    address: "21 Somerset Road, Cape Town, 8001",
    supplierVat: "4770099887",
    number: "SOC-114",
    date: "2026-03-31",
    dateText: "31 March 2026",
    dueText: "14 April 2026",
    pages: 1,
    kind: "invoice",
    currency: "ZAR",
    status: "done",
    note: "One line; the same file comes back in the batch replay.",
    lines: [
      demoLine(1, "Office cleaning, March", 1, 230000, 230000, 34500, 264500),
    ],
    net: 230000,
    vat: 34500,
    gross: 264500,
  },
  {
    id: "bst-221",
    file: "bergsig-bst-221.pdf",
    supplier: "Bergsig Schools Trust",
    address: "5 Bergsig Avenue, Durbanville, 7550",
    supplierVat: null,
    number: "BST-221",
    date: "2026-04-02",
    dateText: "2 April 2026",
    dueText: "16 April 2026",
    pages: 1,
    kind: "invoice",
    currency: "ZAR",
    status: "needs_review",
    held: "figure",
    note: "An exempt tuition invoice whose reading is one rand off the page.",
    lines: [
      demoLine(1, "Tuition fees, term 2", 1, 1250000, 1250000, 0, 1250000, "exempt"),
    ],
    net: 1250000,
    vat: 0,
    gross: 1250000,
    grossRead: 1250100,
    grossPrinted: 1250000,
    reason: "The reading says R 12 501.00. The page prints R 12 500.00, and net plus VAT is R 12 500.00.",
    check: { label: "Total differs", page: 1, sentence: "The reading's gross is R 12 501.00. The page prints R 12 500.00." },
    checks: [
      ["Line total", "Passed", "One line: tuition for one term at R 12 500.00."],
      ["VAT", "Passed", "Tuition is exempt, so no VAT is charged."],
      ["Net plus VAT", "Differs", "R 12 500.00 plus R 0.00 is R 12 500.00, but the reading says R 12 501.00."],
      ["Figure on the page", "Differs", "R 12 501.00 is not a figure the page prints."],
    ],
    confirmedChecks: [
      ["Line total", "Passed", "One line: tuition for one term at R 12 500.00."],
      ["VAT", "Passed", "Tuition is exempt, so no VAT is charged."],
      ["Net plus VAT", "Passed", "R 12 500.00 plus R 0.00 is R 12 500.00."],
      ["Figure on the page", "Passed", "R 12 500.00 appears on page 1."],
    ],
    report: [
      ["What this is", "Bergsig Schools Trust invoiced Kloof Street Coffee Roasters (Pty) Ltd for tuition, invoice BST-221, dated 2 April 2026."],
      ["What was read", "One exempt line: tuition for the second term, R 12 500.00."],
      ["Why it waits", "The reading gives the total as R 12 501.00. Net plus VAT is R 12 500.00, and the page prints R 12 500.00, so the figures do not agree."],
    ],
    reportConfirmed: [
      ["What this is", "Bergsig Schools Trust invoiced Kloof Street Coffee Roasters (Pty) Ltd for tuition, invoice BST-221, dated 2 April 2026."],
      ["What was read", "One exempt line: tuition for the second term, R 12 500.00."],
      ["What was checked", "With the printed total of R 12 500.00, net plus VAT agrees exactly and the page's figure is the one recorded."],
    ],
  },
  {
    id: "ngc-7781",
    file: "nairobi-ngc-7781.pdf",
    supplier: "Nairobi Green Coffee Ltd",
    address: "Ngong Road, Nairobi, Kenya",
    supplierVat: null,
    number: "NGC-7781",
    date: "2026-04-07",
    dateText: "7 April 2026",
    dueText: "7 May 2026",
    pages: 1,
    kind: "invoice",
    currency: "USD",
    status: "needs_review",
    held: "currency",
    note: "A foreign supplier's invoice in US dollars, with no South African VAT.",
    lines: [
      demoLine(1, "Green coffee, 60 kg", 60, 4000, 240000, 0, 240000, "out_of_scope"),
    ],
    net: 240000,
    vat: 0,
    gross: 240000,
    taxNote: "VAT (none)",
    reason: "A foreign invoice in US dollars. No South African VAT is on it; keep it in its own currency.",
    check: { label: "Foreign currency", page: null, sentence: "A foreign invoice in US dollars. It is never added to rand totals." },
    checks: [
      ["Line total", "Passed", "One line: 60 kg at USD 40.00 is USD 2 400.00."],
      ["Currency", "Differs", "The invoice is in US dollars, not rand."],
      ["South African VAT", "Differs", "No South African VAT is on this invoice."],
      ["Net plus VAT", "Passed", "USD 2 400.00 plus USD 0.00 is USD 2 400.00."],
    ],
    confirmedChecks: [
      ["Line total", "Passed", "One line: 60 kg at USD 40.00 is USD 2 400.00."],
      ["Currency", "Passed", "Kept in US dollars and never added to rand totals."],
      ["South African VAT", "Passed", "None is claimed."],
      ["Net plus VAT", "Passed", "USD 2 400.00 plus USD 0.00 is USD 2 400.00."],
    ],
    report: [
      ["What this is", "Nairobi Green Coffee Ltd invoiced Kloof Street Coffee Roasters (Pty) Ltd USD 2 400.00 on 7 April 2026, invoice NGC-7781."],
      ["What was read", "One line: 60 kg of green coffee at USD 40.00."],
      ["Why it waits", "The invoice is in US dollars and carries no South African VAT. A person decides to keep it as a foreign-currency expense."],
    ],
    reportConfirmed: [
      ["What this is", "Nairobi Green Coffee Ltd invoiced Kloof Street Coffee Roasters (Pty) Ltd USD 2 400.00 on 7 April 2026, invoice NGC-7781."],
      ["What was read", "One line: 60 kg of green coffee at USD 40.00."],
      ["What was decided", "Kept as a foreign-currency expense. It stays in US dollars and is never added to rand totals."],
    ],
  },
  {
    id: "crs-8840",
    file: "cape-roasters-crs-8840.pdf",
    supplier: "Cape Roasters Supply",
    address: "44 Marine Drive, Paarden Eiland, 7405",
    supplierVat: "4880123456",
    number: "CRS-8840",
    date: "2026-04-18",
    dateText: "18 April 2026",
    dueText: "",
    pages: 1,
    kind: "credit_note",
    currency: "ZAR",
    status: "done",
    note: "A credit note, kept in its own group and not netted against invoices.",
    lines: [
      demoLine(1, "Credit for short delivery, 18 April", 1, 285000, 285000, 42750, 327750),
    ],
    net: 285000,
    vat: 42750,
    gross: 327750,
  },
  {
    id: "rs-2209",
    file: "roastery-spares-rs-2209.pdf",
    supplier: "Roastery Spares CC",
    address: "7 Victoria Road, Woodstock, 7925",
    supplierVat: "4550033445",
    number: "RS-2209",
    date: "2026-04-22",
    dateText: "22 April 2026",
    dueText: "6 May 2026",
    pages: 1,
    kind: "invoice",
    currency: "ZAR",
    status: "done",
    note: "A simplified tax invoice under R 5 000.",
    lines: [
      demoLine(1, "Drum bearing kit", 1, 120000, 120000, 18000, 138000),
    ],
    net: 120000,
    vat: 18000,
    gross: 138000,
  },
  {
    id: "bb-6174",
    file: "buchu-bean-bb-6174.pdf",
    supplier: "Buchu & Bean Hardware",
    address: "88 Lower Main Road, Observatory, 7925",
    supplierVat: "4330022110",
    number: "BB-6174",
    date: "2026-05-06",
    dateText: "6 May 2026",
    dueText: "",
    pages: 1,
    kind: "invoice",
    currency: "ZAR",
    status: "done",
    note: "A till slip whose total includes VAT.",
    lines: [
      demoLine(1, "Coffee filters", 1, 11500, 10000, 1500, 11500, "standard", true),
      demoLine(1, "Cleaning cloths", 1, 22750, 19783, 2967, 22750, "standard", true),
    ],
    net: 29783,
    vat: 4467,
    gross: 34250,
  },
  {
    id: "pp-0091",
    file: "paper-people-pp-0091.pdf",
    supplier: "Paper People",
    address: "Unit 4, Montague Park, Montague Gardens, 7441",
    supplierVat: "4112233445",
    number: "PP-0091",
    date: "2026-05-14",
    dateText: "14 May 2026",
    dueText: "13 June 2026",
    pages: 4,
    kind: "invoice",
    currency: "ZAR",
    status: "needs_review",
    held: "line",
    note: "Four pages and 45 lines; one line is a cent out.",
    lines: [
      demoLine(1, "Kraft bags", 400, 1200, 480000, 72000, 552000),
      demoLine(1, "Window bags", 300, 960, 288000, 43200, 331200),
      demoLine(1, "Bakery boxes", 200, 1900, 380000, 57000, 437000),
      demoLine(1, "Cup sleeves", 500, 240, 120000, 18000, 138000),
      demoLine(1, "Napkins", 1000, 120, 120000, 18000, 138000),
      demoLine(1, "Stirrers", 500, 80, 40000, 6000, 46000),
      demoLine(1, "Takeaway cups 8oz", 1000, 600, 600000, 90000, 690000),
      demoLine(1, "Takeaway cups 12oz", 500, 720, 360000, 54000, 414000),
      demoLine(1, "Lids 8oz", 1000, 200, 200000, 30000, 230000),
      demoLine(1, "Lids 12oz", 1000, 240, 240000, 36000, 276000),
      demoLine(1, "Paper straws", 400, 160, 64000, 9600, 73600),
      demoLine(1, "Coffee filters", 120, 1400, 168000, 25200, 193200),
      demoLine(2, "Flat lids", 400, 180, 72000, 10800, 82800),
      demoLine(2, "Cup carriers", 100, 480, 48000, 7200, 55200),
      demoLine(2, "Paper bags small", 300, 440, 132000, 19800, 151800),
      demoLine(2, "Paper bags large", 200, 680, 136000, 20400, 156400),
      demoLine(2, "Greaseproof sheets", 500, 140, 70000, 10500, 80500),
      demoLine(2, "Foil wrap", 200, 360, 72000, 10800, 82800),
      demoLine(2, "Cake boxes", 160, 1240, 198400, 29760, 228160),
      demoLine(2, "Pastry bags", 200, 300, 60000, 9000, 69000),
      demoLine(2, "Stickers", 500, 120, 60000, 9000, 69000),
      demoLine(2, "Tape", 24, 1100, 26400, 3960, 30360),
      demoLine(2, "Labels", 400, 170, 68000, 10200, 78200),
      demoLine(2, "Receipt rolls", 50, 760, 38000, 5700, 43700),
      demoLine(3, "Wooden stirrers", 400, 110, 44000, 6600, 50600),
      demoLine(3, "Sugar sachets", 600, 80, 48000, 7200, 55200),
      demoLine(3, "Sweetener sachets", 300, 90, 27000, 4050, 31050),
      demoLine(3, "Tea bags", 200, 190, 38000, 5700, 43700),
      demoLine(3, "Hot chocolate sachets", 100, 480, 48000, 7200, 55200),
      demoLine(3, "Cleaning cloths", 80, 300, 24000, 3600, 27600),
      demoLine(3, "Degreaser", 6, 1700, 10200, 1530, 11730),
      demoLine(3, "Bin liners", 100, 240, 24000, 3600, 27600),
      demoLine(3, "Hand soap", 12, 900, 10800, 1620, 12420),
      demoLine(3, "Sanitiser", 6, 1560, 9360, 1404, 10764),
      demoLine(3, "Gloves", 300, 180, 54000, 8100, 62100),
      demoLine(3, "Hairnets", 150, 140, 21000, 3150, 24150),
      demoLine(4, "Compostable bowls", 3, 41250, 123749, 18562, 142311),
      demoLine(4, "Bowl lids", 150, 280, 42000, 6300, 48300),
      demoLine(4, "Wooden forks", 300, 150, 45000, 6750, 51750),
      demoLine(4, "Napkin dispensers", 12, 2900, 34800, 5220, 40020),
      demoLine(4, "Menu holders", 20, 1900, 38000, 5700, 43700),
      demoLine(4, "Loyalty cards", 200, 220, 44000, 6600, 50600),
      demoLine(4, "Gift boxes", 30, 1640, 49200, 7380, 56580),
      demoLine(4, "Ribbon", 40, 520, 20800, 3120, 23920),
      demoLine(4, "Packing tape", 20, 1247, 24940, 3741, 28681),
    ],
    net: 4821649,
    vat: 723247,
    gross: 5544896,
    reason: "Line 37 is 3 × R 412.50 = R 1 237.50. The page prints R 1 237.49, so the line is one cent out.",
    check: { label: "Line arithmetic", page: 4, sentence: "Line 37 is one cent below 3 × R 412.50." },
    checks: [
      ["Every line but one", "Passed", "44 lines are quantity times unit price to the cent."],
      ["Line 37", "Differs", "3 × R 412.50 is R 1 237.50; the page prints R 1 237.49."],
      ["VAT", "Passed", "R 7 232.47 on the printed net of R 48 216.49."],
      ["Net plus VAT", "Passed", "R 48 216.49 plus R 7 232.47 is R 55 448.96, as printed."],
    ],
    confirmedChecks: [
      ["Every line but one", "Passed", "44 lines are quantity times unit price to the cent."],
      ["Line 37", "Passed", "Accepted as printed: R 1 237.49."],
      ["VAT", "Passed", "R 7 232.47 on the printed net of R 48 216.49."],
      ["Net plus VAT", "Passed", "R 48 216.49 plus R 7 232.47 is R 55 448.96, as printed."],
    ],
    report: [
      ["What this is", "Paper People invoiced Kloof Street Coffee Roasters (Pty) Ltd R 55 448.96 on 14 May 2026, invoice PP-0091."],
      ["What was read", "45 packaging lines over four pages. The invoice's totals agree with its printed lines."],
      ["Why it waits", "Line 37 is 3 boxes at R 412.50, which is R 1 237.50, printed as R 1 237.49. The line is one cent out."],
    ],
    reportConfirmed: [
      ["What this is", "Paper People invoiced Kloof Street Coffee Roasters (Pty) Ltd R 55 448.96 on 14 May 2026, invoice PP-0091."],
      ["What was read", "45 packaging lines over four pages. The invoice's totals agree with its printed lines."],
      ["What was decided", "Line 37 is accepted as printed, R 1 237.49. The totals are unchanged."],
    ],
  },
  {
    id: "cs-441",
    file: "cape-stationery-cs-441.pdf",
    supplier: "Cape Stationery",
    address: "17 Loop Street, Cape Town, 8001",
    supplierVat: "4998877665",
    number: "CS-441",
    date: null,
    dateText: "No date",
    dueText: "",
    pages: 1,
    kind: "invoice",
    currency: "ZAR",
    status: "needs_review",
    held: "date",
    note: "No date on the page; the delivery note is dated 12 May.",
    lines: [
      demoLine(1, "Office stationery, order 441", 1, 64000, 64000, 9600, 73600),
    ],
    net: 64000,
    vat: 9600,
    gross: 73600,
    reason: "This invoice has no date. The delivery note packed with it is dated 12 May 2026.",
    check: { label: "No date", page: null, sentence: "No date on the page. The delivery note is dated 12 May 2026." },
    checks: [
      ["Page fields", "Differs", "The page has no invoice date."],
      ["Line total", "Passed", "One line at R 640.00."],
      ["VAT", "Passed", "R 96.00 on R 640.00 at 15%."],
      ["Net plus VAT", "Passed", "R 640.00 plus R 96.00 is R 736.00."],
    ],
    confirmedChecks: [
      ["Date", "Passed", "Taken from the delivery note: 12 May 2026."],
      ["Line total", "Passed", "One line at R 640.00."],
      ["VAT", "Passed", "R 96.00 on R 640.00 at 15%."],
      ["Net plus VAT", "Passed", "R 640.00 plus R 96.00 is R 736.00."],
    ],
    report: [
      ["What this is", "Cape Stationery invoiced Kloof Street Coffee Roasters (Pty) Ltd R 736.00, invoice CS-441."],
      ["What was read", "One standard line: office stationery, R 640.00 plus R 96.00 VAT."],
      ["Why it waits", "The page has no date. The delivery note packed with it is dated 12 May 2026, so a person confirms which date the invoice takes."],
    ],
    reportConfirmed: [
      ["What this is", "Cape Stationery invoiced Kloof Street Coffee Roasters (Pty) Ltd R 736.00, invoice CS-441, dated 12 May 2026."],
      ["What was read", "One standard line: office stationery, R 640.00 plus R 96.00 VAT."],
      ["What was decided", "The delivery note's date, 12 May 2026, is recorded, so the invoice lands in May."],
    ],
  },
  {
    id: "cp-3301",
    file: "cape-packaging-cp-3301.pdf",
    supplier: "Cape Packaging",
    address: "3 Neptune Street, Paarden Eiland, 7405",
    supplierVat: "4556677889",
    number: "CP-3301",
    date: "2026-04-09",
    dateText: "9 April 2026",
    dueText: "23 April 2026",
    pages: 1,
    kind: "invoice",
    currency: "ZAR",
    status: "done",
    note: "Cups, lids and napkins, with VAT at 15%.",
    lines: [
      demoLine(1, "Paper cups", 1000, 900, 900000, 135000, 1035000),
      demoLine(1, "Lids", 1000, 250, 250000, 37500, 287500),
      demoLine(1, "Napkins", 1000, 145, 145000, 21750, 166750),
    ],
    net: 1295000,
    vat: 194250,
    gross: 1489250,
  },
  {
    id: "cc-7712",
    file: "cape-couriers-cc-7712.pdf",
    supplier: "Cape Couriers CC",
    address: "2 Sable Road, Maitland, 7405",
    supplierVat: "4667788990",
    number: "CC-7712",
    date: "2026-04-30",
    dateText: "30 April 2026",
    dueText: "14 May 2026",
    pages: 1,
    kind: "invoice",
    currency: "ZAR",
    status: "done",
    note: "Local and after-hours deliveries.",
    lines: [
      demoLine(1, "Local deliveries", 60, 3500, 210000, 31500, 241500),
      demoLine(1, "After-hours deliveries", 11, 9000, 99000, 14850, 113850),
    ],
    net: 309000,
    vat: 46350,
    gross: 355350,
  },
  {
    id: "ksp-0331",
    file: "kloof-properties-ksp-0331.pdf",
    supplier: "Kloof Street Properties CC",
    address: "88 Kloof Street, Gardens, 8001",
    supplierVat: "4223344556",
    number: "KSP-0331",
    date: "2026-05-01",
    dateText: "1 May 2026",
    dueText: "1 May 2026",
    pages: 1,
    kind: "invoice",
    currency: "ZAR",
    status: "done",
    note: "Commercial rent for May, with VAT at 15%.",
    lines: [
      demoLine(1, "Commercial rent, May 2026", 1, 1850000, 1850000, 277500, 2127500),
    ],
    net: 1850000,
    vat: 277500,
    gross: 2127500,
  },
  {
    id: "bse-4410",
    file: "bright-spark-bse-4410.pdf",
    supplier: "Bright Spark Electrical CC",
    address: "19 Voortrekker Road, Goodwood, 7460",
    supplierVat: "4776655443",
    number: "BSE-4410",
    date: "2026-05-20",
    dateText: "20 May 2026",
    dueText: "3 June 2026",
    pages: 1,
    kind: "invoice",
    currency: "ZAR",
    status: "done",
    note: "A repair, with parts and labour.",
    lines: [
      demoLine(1, "Parts, replacement element", 1, 98000, 98000, 14700, 112700),
      demoLine(1, "Labour, 5 hours", 5, 22000, 110000, 16500, 126500),
    ],
    net: 208000,
    vat: 31200,
    gross: 239200,
  },
  {
    id: "rs-2215",
    file: "roastery-spares-rs-2215.pdf",
    supplier: "Roastery Spares CC",
    address: "7 Victoria Road, Woodstock, 7925",
    supplierVat: "4550033445",
    number: "RS-2215",
    date: "2026-05-29",
    dateText: "29 May 2026",
    dueText: "12 June 2026",
    pages: 1,
    kind: "invoice",
    currency: "ZAR",
    status: "processing",
    note: "Uploaded minutes ago and still reading.",
    lines: [],
  },
];

// Confirmations live only in this page's memory. A reload, Start over or Exit demo clears them.
const demoConfirmed = new Map();

function demoDocs() {
  return DEMO_DOCUMENTS.map(demoResolve);
}

function demoDoc(id) {
  return DEMO_DOCUMENTS.find((doc) => doc.id === id) || null;
}

function demoResolve(doc) {
  const done = demoConfirmed.get(doc.id);
  return done ? { ...doc, ...done, status: "done", confirmed: true } : doc;
}

function demoHeldResolution(doc) {
  if (doc.held === "figure") return { gross: doc.grossPrinted };
  if (doc.held === "currency") return { foreignCurrency: true };
  if (doc.held === "line") return { acceptedLine: 37 };
  return { date: "2026-05-12", dateText: "12 May 2026" };
}

function demoReset() {
  demoConfirmed.clear();
}

function demoMoney(minor, currency) {
  if (minor == null) return "–";
  const sign = minor < 0 ? "-" : "";
  const abs = Math.abs(minor);
  const whole = String(Math.floor(abs / 100)).replace(/\B(?=(\d{3})+(?!\d))/g, " ");
  const amount = whole + "." + String(abs % 100).padStart(2, "0");
  return (currency === "USD" ? "USD " : "R ") + sign + amount;
}

function demoKind(doc) {
  return DEMO_KIND[doc.kind] || doc.kind;
}

// BST's row shows the reading's total until a person confirms the printed one.
function demoStatedGross(doc) {
  if (doc.status === "processing") return null;
  return doc.status === "needs_review" && doc.held === "figure" ? doc.grossRead : doc.gross;
}

function demoChecks(doc) {
  if (doc.confirmed && doc.confirmedChecks) return doc.confirmedChecks;
  return doc.checks || demoChecksFor(doc);
}

function demoChecksFor(doc) {
  return [
    ["Every line", "Passed", doc.lines.length === 1 ? "The one line agrees with the page." : "Each of the " + doc.lines.length + " lines agrees with the page."],
    ["VAT", "Passed", doc.vat === 0 ? "No VAT is charged." : "VAT agrees with the document's totals."],
    ["Net plus VAT", "Passed", demoMoney(doc.net, doc.currency) + " plus " + demoMoney(doc.vat, doc.currency) + " is " + demoMoney(doc.gross, doc.currency) + "."],
    ["Figures on the page", "Passed", "Every amount recorded appears on the page."],
  ];
}

function demoReport(doc) {
  if (doc.confirmed && doc.reportConfirmed) return doc.reportConfirmed;
  return doc.report || demoReportFor(doc);
}

function demoReportFor(doc) {
  const kind = doc.kind === "credit_note" ? "credit note" : doc.kind === "unclassified" ? "document" : "invoice";
  return [
    ["What this is", doc.supplier + " sent Kloof Street Coffee Roasters (Pty) Ltd " + demoMoney(doc.gross, doc.currency) + " on " + doc.dateText + ", " + kind + " " + doc.number + "."],
    ["What was read", doc.lines.length + " line" + (doc.lines.length === 1 ? "" : "s") + " over " + doc.pages + " page" + (doc.pages === 1 ? "" : "s") + "."],
    ["What was checked", "Every line, the VAT and net plus VAT agree with the page."],
  ];
}

function demoStrip() {
  return h("div", { class: "demo-strip" },
    h("span", { class: "demo-chip" }, "Demo"),
    h("span", { class: "demo-strip-text" }, DEMO_NOTE),
    h("span", { class: "demo-actions" },
      h("button", { type: "button", class: "link", onclick: () => { demoReset(); route(); } }, "Start over"),
      h("a", { class: "link", href: "#/demo" }, "Back to the demo"),
      h("button", { type: "button", class: "link", onclick: () => { stopPolling(); demoReset(); location.hash = "#/"; } }, "Exit demo")));
}

function demoCard(doc) {
  const gross = demoStatedGross(doc);
  return h("a", { class: "sample-card", href: "#/demo/invoice/" + doc.id },
    h("span", { class: "sample-file" }, doc.file),
    h("strong", {}, doc.supplier),
    h("span", { class: "small muted" }, demoKind(doc) + " " + doc.number + " · " + doc.dateText + " · " + (gross == null ? "–" : demoMoney(gross, doc.currency))),
    badge(doc.status),
    h("span", { class: "small" }, doc.note));
}

const DEMO_FEATURED = ["pp-0091", "bst-221", "crs-8840"];

const DEMO_LIBRARY = [
  ["Everyday invoices", ["tp-4410", "crs-8821", "wf-55302", "soc-114", "cp-3301", "cc-7712", "ksp-0331", "bse-4410"]],
  ["Different tax treatments", ["dd-1740", "mf-8092", "rs-2209", "bb-6174", "ngc-7781"]],
  ["Credit notes", ["crs-8840"]],
  ["The hard ones", ["bst-221", "pp-0091", "cs-441", "rs-2215"]],
];

function demoHome() {
  stopPolling();
  const featured = DEMO_FEATURED.map((id) => demoCard(demoResolve(demoDoc(id))));
  const library = DEMO_LIBRARY.map(([title, ids]) =>
    h("section", { class: "library-group" },
      h("h2", {}, title),
      h("div", { class: "library-grid" }, ids.map((id) => demoCard(demoResolve(demoDoc(id)))))));
  show(h("div", { class: "page demo" },
    demoStrip(),
    h("div", { class: "page-head" },
      h("p", { class: "eyebrow" }, "Demo"),
      h("h1", {}, "A quarter in the life of an inbox."),
      h("p", { class: "lede" }, "Eighteen supplier documents from March to May 2026, prepared so you can walk the whole workspace without a key: the inbox, the review queue, the register and a batch upload. No model is called, nothing is uploaded and nothing is saved."),
      h("p", { class: "small muted" }, "Kloof Street Coffee Roasters (Pty) Ltd is a made-up roastery in Cape Town. Every supplier, person and number here is fictional.")),
    h("div", { class: "card" },
      h("p", {}, "This sample is built to show the hard cases: a total off by a rand, a line off by a cent, a foreign invoice and an undated page. 13 of the 17 finished documents needed nobody (76.5%); a clean month runs higher.")),
    h("section", { class: "demo-featured" },
      h("h2", {}, "Start with these"),
      h("div", { class: "demo-featured-grid" }, featured)),
    h("section", { class: "library" }, library),
    h("p", { class: "small muted" }, "In the real app a model reads each page, every figure is checked against the page it came from, and what cannot be settled goes to a person. ClearCut is invite-only while in alpha: ask for a key from whoever sent you this link.")));
}

function demoJourney(id, withTimeline) {
  stopPolling();
  const base = demoDoc(id);
  if (!base) {
    location.hash = "#/demo";
    return;
  }
  const state = { phase: "document", focus: false, redraw: null };
  const stage = h("div", {});
  const initial = () => {
    if (withTimeline) return "timeline";
    if (base.status === "processing") return "reading";
    if (base.status === "needs_review" && !demoConfirmed.has(base.id)) return "review";
    return "document";
  };
  state.phase = initial();
  const draw = () => {
    const doc = demoResolve(base);
    let node;
    if (state.phase === "timeline") {
      node = demoTimeline(doc, () => {
        state.phase = doc.status === "processing" ? "reading" : "document";
        draw();
      });
    } else if (state.phase === "review") {
      node = demoReview(doc, state, draw);
    } else if (state.phase === "reading") {
      node = demoReading(doc);
    } else {
      node = demoDocumentScreen(doc, state, draw);
    }
    stage.replaceChildren(node);
    if (state.focus) {
      const banner = node.querySelector(".banner.ok");
      if (banner) banner.focus();
      state.focus = false;
    }
  };
  state.redraw = draw;
  show(h("div", { class: "page demo" },
    demoStrip(),
    h("p", { class: "small" }, h("a", { href: "#/demo" }, "← Back to the demo")),
    stage));
  draw();
}

function demoTimeline(doc, done) {
  const steps = ["File accepted", doc.pages + (doc.pages === 1 ? " page found" : " pages found")];
  for (let page = 1; page <= doc.pages; page += 1) steps.push("Reading page " + page + " of " + doc.pages);
  if (doc.status !== "processing") {
    steps.push("Checking every figure", doc.status === "needs_review" ? "Needs review" : "Done");
  }
  const list = h("ol", { class: "timeline", "aria-label": "What the demo is doing" });
  const status = h("p", { class: "small muted", role: "status" }, "");
  const progress = h("progress", { class: "demo-progress", max: steps.length, value: 0 });
  let at = 0;
  const advance = () => {
    if (at === steps.length) {
      timer = setTimeout(done, 450);
      return;
    }
    list.append(h("li", {}, steps[at]));
    status.textContent = steps[at];
    progress.value = at + 1;
    at += 1;
    timer = setTimeout(advance, at < 2 ? 700 : 1000);
  };
  advance();
  return h("div", { class: "card" },
    h("h1", {}, doc.file),
    h("p", { class: "small muted" }, "This timeline is simulated. No model is called and no file is uploaded."),
    progress,
    status,
    list,
    h("div", { class: "actions" },
      h("button", { type: "button", class: "secondary", onclick: () => { stopPolling(); done(); } }, "Skip to the result")));
}

function demoReading(doc) {
  return h("div", {},
    h("div", { class: "row" }, h("h1", {}, doc.file), badge("processing")),
    h("p", { class: "small muted" }, doc.supplier + " · " + demoKind(doc) + " " + doc.number + " · " + doc.dateText + " · prepared sample"),
    h("div", { class: "card" },
      h("h2", {}, "Still reading."),
      h("p", {}, "The pages have been read, but the figures are not finished yet. There is nothing to review or export until the reading finishes."),
      h("p", { class: "small muted" }, "In the real app this waits on the worker and the model. Here it stays like this on purpose: the demo cannot finish a reading it never started.")),
    demoEndCard());
}

function demoChecklist(rows) {
  return h("div", { class: "checks" },
    rows.map(([label, verdict, detail]) =>
      h("div", { class: "check " + (verdict === "Passed" ? "pass" : "fail") },
        h("span", { class: "check-verdict" }, verdict),
        h("strong", {}, label),
        h("span", { class: "small muted" }, detail))));
}

function demoConfirmationNote(doc) {
  if (doc.held === "figure") return "The printed total " + demoMoney(doc.grossPrinted, doc.currency) + " is recorded.";
  if (doc.held === "date") return "The invoice is dated " + doc.dateText + ", from the delivery note.";
  if (doc.held === "line") return "Line 37 is accepted as printed.";
  if (doc.held === "currency") return "It stays in US dollars and is never added to rand totals.";
  return "The document is done.";
}

function demoDocumentScreen(doc, state, redraw) {
  const held = doc.status === "needs_review";
  const parts = [
    h("div", { class: "row" }, h("h1", {}, doc.file), badge(doc.status)),
    h("p", { class: "small muted" },
      doc.supplier + " · " + demoKind(doc) + " " + doc.number + " · " + doc.dateText + " · " +
      doc.pages + (doc.pages === 1 ? " page" : " pages") + " · prepared sample"),
  ];
  if (held) {
    parts.push(
      h("div", { class: "banner", role: "note" },
        h("strong", {}, "A person needs to look at this document. "),
        doc.reason,
        h("br"),
        h("span", { class: "small muted" }, "Nothing was sent to a model and nothing is stored: this is the demo's prepared reading."),
        h("div", { class: "actions" },
          h("button", { type: "button", onclick: () => { state.phase = "review"; redraw(); } }, "Review this page"))));
  }
  if (doc.confirmed) {
    parts.push(
      h("div", { class: "banner ok", role: "status", tabindex: "-1" },
        h("strong", {}, "Confirmed by you in this demo. "),
        demoConfirmationNote(doc)));
  }
  parts.push(h("h2", {}, "Figures checked"), demoChecklist(demoChecks(doc)));
  parts.push(h("h2", {}, "Report"), h("div", { class: "report" },
    demoReport(doc).map(([heading, text]) => h("div", {}, h("h3", {}, heading), h("p", {}, text)))));
  if (!held) parts.push(h("h2", {}, "Export"), demoExportPanel(doc));
  parts.push(demoEndCard());
  return h("div", {}, parts);
}

function demoConfirmLabel(doc) {
  if (doc.held === "figure") return "Confirm " + demoMoney(doc.grossPrinted, doc.currency);
  if (doc.held === "currency") return "Keep it in US dollars";
  if (doc.held === "line") return "Accept the printed line";
  return "Use 12 May 2026";
}

function demoReviewSheet(doc) {
  const lines = doc.held === "line" ? doc.lines.filter((line) => line.page === 4) : doc.lines;
  const rows = lines.map((line) => {
    const number = doc.lines.indexOf(line) + 1;
    const amount = doc.held === "line" && number === 37
      ? h("mark", {}, demoMoney(line.net, doc.currency))
      : demoMoney(line.net, doc.currency);
    return h("tr", {},
      h("td", {}, doc.held === "line" ? "Line " + number + " · " + line.description : line.description),
      h("td", { class: "num" }, line.quantity),
      h("td", { class: "num" }, demoMoney(line.unit, doc.currency)),
      h("td", { class: "num" }, amount));
  });
  return h("div", { class: "invoice-sheet", "aria-label": "Sample document " + doc.number },
    h("div", { class: "invoice-head" },
      h("div", {},
        h("strong", {}, doc.supplier),
        h("span", { class: "small muted" }, doc.address),
        h("span", { class: "small muted" }, doc.supplierVat ? "VAT " + doc.supplierVat : "No South African VAT number")),
      h("div", { class: "invoice-meta" },
        h("span", {}, demoKind(doc) + " " + doc.number),
        h("span", {}, doc.date ? doc.dateText : "No date printed"),
        doc.dueText ? h("span", {}, "Due " + doc.dueText) : null)),
    h("table", {},
      h("thead", {}, h("tr", {},
        h("th", {}, "Description"), h("th", { class: "num" }, "Qty"),
        h("th", { class: "num" }, "Unit"), h("th", { class: "num" }, "Amount"))),
      h("tbody", {}, rows)),
    h("div", { class: "invoice-totals" },
      h("div", {}, h("span", {}, "Net"), h("b", {}, demoMoney(doc.net, doc.currency))),
      h("div", {}, h("span", {}, doc.taxNote || "VAT"), h("b", {}, demoMoney(doc.vat, doc.currency))),
      h("div", { class: "due" }, h("span", {}, "Total"), h("b", {},
        doc.held === "figure" ? h("mark", {}, demoMoney(doc.grossPrinted, doc.currency)) : demoMoney(doc.gross, doc.currency)))));
}

function demoReviewReading(doc) {
  const fields = h("dl", { class: "reading" });
  fields.append(h("dt", {}, "Document number"), h("dd", {}, doc.number));
  if (doc.held === "date") {
    fields.append(h("dt", {}, "Date"), h("dd", { class: "error" }, "Not found"));
  } else {
    fields.append(h("dt", {}, "Date"), h("dd", {}, doc.dateText));
  }
  if (doc.held === "figure") {
    fields.append(h("dt", {}, "Total (gross)"), h("dd", { class: "error" }, demoMoney(doc.grossRead, doc.currency)));
  }
  if (doc.held === "currency") {
    fields.append(h("dt", {}, "Currency"), h("dd", {}, "US dollars"));
    fields.append(h("dt", {}, "South African VAT"), h("dd", {}, "None"));
  }
  if (doc.held === "line") {
    fields.append(h("dt", {}, "Net"), h("dd", {}, demoMoney(doc.net, doc.currency)));
    fields.append(h("dt", {}, "Line 37"), h("dd", { class: "error" },
      "3 × " + demoMoney(41250, doc.currency) + " = " + demoMoney(123750, doc.currency)));
  }
  return [
    h("h2", {}, "What the reader says"),
    fields,
    h("div", { class: "banner", role: "note" },
      h("strong", {}, "What needs a person. "),
      doc.reason),
    doc.held === "date"
      ? h("div", { class: "card" },
          h("h3", {}, "The delivery note"),
          h("p", {}, "Packed with the invoice and dated 12 May 2026. The review uses that date."))
      : null,
  ];
}

function demoReview(doc, state, redraw) {
  const back = () => { state.phase = "document"; redraw(); };
  return h("div", {},
    h("div", { class: "row" }, h("h1", {}, "Review " + doc.file), badge("needs_review")),
    h("p", { class: "small muted" }, "This page is a recreation of the made-up sample document, so it reads the same in dark mode."),
    h("div", { class: "review-split" },
      demoReviewSheet(doc),
      h("div", { class: "card" }, demoReviewReading(doc),
        h("p", { class: "small muted" }, "In the real app this is a form: the correction is checked by the same rules as a model's reading, recorded as yours and costs nothing."),
        h("div", { class: "row" },
          h("button", {
            type: "button",
            onclick: () => {
              demoConfirmed.set(doc.id, demoHeldResolution(doc));
              state.phase = "document";
              state.focus = true;
              redraw();
            },
          }, demoConfirmLabel(doc)),
          h("button", { type: "button", class: "secondary", onclick: back }, "Back to the document")))));
}

function demoCsvCell(value) {
  let text = String(value == null ? "" : value);
  if (/^[=+\-@\t\r]/.test(text)) text = "'" + text;
  return '"' + text.replaceAll('"', '""') + '"';
}

function demoExport(doc, format) {
  if (format === "csv") {
    const rows = [[demoCsvCell("ClearCut demo. Sample data, not a real invoice.")]];
    rows.push(["supplier", "document", "date", "page", "description", "quantity", "unit", "line_net", "line_vat", "line_gross", "tax"].map(demoCsvCell).join(","));
    for (const line of doc.lines) {
      rows.push([doc.supplier, doc.number, doc.dateText, line.page, line.description, line.quantity,
        demoMoney(line.unit, doc.currency), demoMoney(line.net, doc.currency),
        demoMoney(line.vat, doc.currency), demoMoney(line.gross, doc.currency), DEMO_TAX[line.tax]].map(demoCsvCell).join(","));
    }
    rows.push(["", "", "", "", "Net", "", "", demoMoney(doc.net, doc.currency), "", "", ""].map(demoCsvCell).join(","));
    rows.push(["", "", "", "", "VAT", "", "", demoMoney(doc.vat, doc.currency), "", "", ""].map(demoCsvCell).join(","));
    rows.push(["", "", "", "", "Total", "", "", demoMoney(doc.gross, doc.currency), "", "", ""].map(demoCsvCell).join(","));
    return rows.join("\n");
  }
  const lines = doc.lines.map((line) =>
    "| " + line.page + " | " + line.description + " | " + line.quantity + " | " +
    demoMoney(line.unit, doc.currency) + " | " + demoMoney(line.net, doc.currency) + " | " +
    demoMoney(line.vat, doc.currency) + " | " + demoMoney(line.gross, doc.currency) + " | " +
    DEMO_TAX[line.tax] + " |");
  return [
    "# ClearCut demo: sample data, not a real invoice",
    "",
    "**" + doc.supplier + "** · " + demoKind(doc).toLowerCase() + " " + doc.number + " · " + doc.dateText,
    "",
    "| Page | Description | Qty | Unit | Net | VAT | Line total | Tax |",
    "|---:|---|---:|---:|---:|---:|---:|---|",
    ...lines,
    "",
    "Net " + demoMoney(doc.net, doc.currency),
    "VAT " + demoMoney(doc.vat, doc.currency),
    "**Total " + demoMoney(doc.gross, doc.currency) + "**",
  ].join("\n");
}

function demoExportPanel(doc) {
  const out = h("pre", { class: "demo-output", tabindex: "0" }, "Choose CSV or Markdown.");
  const note = h("p", { class: "small muted", role: "status" }, "");
  const build = (format) => {
    out.textContent = demoExport(doc, format);
    note.textContent = "Built in this page from the sample's figures. No file was requested from the server.";
  };
  const copy = h("button", {
    type: "button", class: "secondary",
    onclick: async () => {
      try {
        await navigator.clipboard.writeText(out.textContent);
        note.textContent = "Copied.";
      } catch (e) {
        note.textContent = "Select the text above and copy it.";
      }
    },
  }, "Copy");
  return h("div", {},
    h("p", { class: "small muted" }, "The real app exports JSON, CSV, Excel, Word, PDF and more. Here these two are built in your browser."),
    h("div", { class: "row" },
      h("button", { type: "button", onclick: () => build("csv") }, "Show CSV"),
      h("button", { type: "button", class: "secondary", onclick: () => build("md") }, "Show Markdown"),
      copy),
    out,
    note);
}

function demoEndCard() {
  return h("div", { class: "card demo-end" },
    h("h2", {}, "That is the demo."),
    h("p", {}, "ClearCut is invite-only while in alpha. Ask for a key from whoever sent you this link, then sign in and add your own Anthropic key in Settings."),
    h("div", { class: "row" },
      h("a", { class: "button", href: "#/" }, "Back to the sign-in page"),
      h("a", { href: REPO, rel: "noreferrer" }, "Read the source")));
}
```

- [ ] **Step 5: Update the router**

Replace the demo part of `route()` (from `if (hash === "#/demo")` through the sample match) with:

```js
  if (hash === "#/demo") return demoHome();
  const journey = hash.match(/^#\/demo\/(invoice|document)\/([a-z0-9-]+)$/);
  if (journey) return demoJourney(journey[2], journey[1] === "invoice");
  const legacy = hash.match(/^#\/demo\/([a-z0-9-]+)$/);
  if (legacy) return demoJourney(legacy[1], true);
```

- [ ] **Step 6: Add the slice 1 CSS**

Insert immediately before `/* ---------------------------------------------------------------- small ---- */` in `app.css`:

```css
.demo-featured { margin: 30px 0; }
.demo-featured h2, .library h2, .library-group h2 { font-family: var(--serif); }
.demo-featured h2 { font-size: 26px; }
.demo-featured-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 16px; }
.library { margin: 34px 0; display: grid; gap: 30px; }
.library-group h2 { font-size: 24px; margin-bottom: 12px; }
.library-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 16px; }
.demo-hero-actions { display: flex; flex-wrap: wrap; gap: 10px; margin: 20px 0 6px; }
@media (max-width: 900px) {
  .demo-featured-grid, .library-grid { grid-template-columns: 1fr 1fr; }
}
@media (max-width: 600px) {
  .demo-featured-grid, .library-grid { grid-template-columns: 1fr; }
}
```

- [ ] **Step 7: Check the script parses**

Run: `node --check crates/clearcut-server/src/web/app.js`

Expected: no output, exit code 0.

- [ ] **Step 8: Run the Rust tests**

Run: `cargo test -p clearcut-server web::tests::the_page_offers_a_demo_that_needs_no_key web::tests::the_demo_keeps_its_prepared_arithmetic`

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/clearcut-server/src/web/app.js crates/clearcut-server/src/web/app.css crates/clearcut-server/src/web.rs
git commit -m "feat(web): the demo becomes a made-up quarter"
```

### Task 2: Prove slice 1, document it, ship it

**Files:**
- Modify: `crates/clearcut-server/src/web.rs` (add the review and isolation tests)
- Modify: `docs/LAUNCH.md:14`
- Modify: `docs/WORKLOG.md` (new entry at the top, under the header)

**Interfaces:**
- Consumes: Task 1.
- Produces: the first green slice and its record.

- [ ] **Step 1: Add the review-kind and isolation tests**

Add to `crates/clearcut-server/src/web.rs`:

```rust
    #[tokio::test]
    async fn the_demo_shows_every_kind_of_review() {
        let (_, _, js) = fetch("/app.js").await;
        for marker in [
            "The reading says R 12 501.00",
            "A foreign invoice in US dollars",
            "Line 37 is 3 × R 412.50",
            "This invoice has no date",
            "Keep it in US dollars",
            "Accept the printed line",
            "Use 12 May 2026",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
    }

    #[tokio::test]
    async fn the_demo_cannot_reach_the_api_or_storage() {
        let (_, _, js) = fetch("/app.js").await;
        let start = js.find("const DEMO_DOCUMENTS").expect("the demo data");
        let end = js
            .find("// ---------------------------------------------------------------- routing ----")
            .expect("the routing marker");
        let demo = &js[start..end];
        for forbidden in [
            "fetch(",
            "api(",
            "Authorization",
            "localStorage",
            "sessionStorage",
            "indexedDB",
            "document.cookie",
        ] {
            assert!(!demo.contains(forbidden), "{forbidden} in the demo");
        }
    }
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p clearcut-server web::tests`

Expected: PASS. If `the_demo_cannot_reach_the_api_or_storage` fails on `api(`, find the offending word inside the demo block and reword the comment; the demo must not contain the token.

- [ ] **Step 3: Walk the demo in a browser**

Start a static server for the web files:

```bash
python -m http.server 8099 --directory crates/clearcut-server/src/web
```

Open `http://127.0.0.1:8099/#/demo` and check:

- The home names Kloof Street Coffee Roasters, the quarter and the 76.5% sentence.
- Four library groups hold 8, 5, 1 and 4 cards; three featured cards.
- `#/demo/invoice/pp-0091` runs the timeline and lands on the document with a held banner.
- `#/demo/tp-4410` and `#/demo/bst-221` still work.
- `#/demo/document/rs-2215` shows "Still reading." and no export.
- Opening the review on BST, NGC, PP and CS shows the right reason and button, and confirming returns to the document with a focused confirmation banner.
- With the network panel open, every request is for the page, `app.js`, `app.css` or a favicon; there is no `/v1` request and no `Authorization` header.

- [ ] **Step 4: Update LAUNCH**

Replace line 14 of `docs/LAUNCH.md` with:

```markdown
- **A public demo** at `#/demo`: a made-up roastery's first quarter, 18 prepared supplier documents from March to May 2026, with a library home and a full journey for every document (timeline, checks, report, four review kinds and a browser-built export). The workspace, register and batch replay follow in the next slices. It needs no key, calls no model, stores nothing and cannot reach a tenant; accounts remain invite-only.
```

- [ ] **Step 5: Add the worklog entry**

Insert below the `_Newest first..._` line in `docs/WORKLOG.md`:

```markdown
## 2026-11-01 (part 82): the demo becomes a made-up quarter
**Done**
- **One fictional SME, one closed quarter.** Kloof Street Coffee Roasters (Pty) Ltd, a made-up Cape Town roastery, receives 18 prepared supplier documents from 1 March to 31 May 2026: standard invoices, a mixed zero-rated and standard dairy invoice, a fine, a credit note, a simplified invoice, a VAT-inclusive till slip, a four-page 45-line invoice, a foreign USD invoice and an undated invoice.
- **A library home.** Four groups by lesson (everyday, different tax treatments, credit notes, the hard ones), three featured documents, and the honest arithmetic: 13 of 17 finished documents needed nobody, 76.5%.
- **A journey for every document.** `#/demo/invoice/<id>` runs the simulated timeline, the document with its checks and report, the review for a held document and a browser-built CSV and Markdown export. `#/demo/document/<id>` opens the document directly, or the review when one is waiting.
- **Four reviews, four reasons.** A total one rand off the page, a foreign-currency invoice, a line one cent out, and a page with no date. Each confirmation is recorded in page memory and returns to the document with the confirmation banner focused.
- **Isolation stays.** No API call, key, tenant, storage, upload or model call in the demo; a Rust test fails if `fetch(`, `api(`, `Authorization` or a storage API enters the demo block.
- **Tests:** `node --check`, the web tests for the data anchors, the four review reasons and the isolation boundary; the demo walked in a browser signed out.
**Not done:** the workspace, review queue, register and batch replay wait for their slices; no real document is used anywhere.
```

- [ ] **Step 6: Run the gate**

```bash
node --check crates/clearcut-server/src/web/app.js
cargo fmt --all
cargo clippy -p clearcut-server --all-targets -- -D warnings
cargo test -p clearcut-server
```

Expected: all pass.

- [ ] **Step 7: Ship the slice**

```bash
git add crates/clearcut-server/src/web.rs docs/LAUNCH.md docs/WORKLOG.md
git commit -m "docs: record the demo quarter and its reviews"
git push -u origin feat/demo-quarter
gh pr create --title "feat(web): the demo becomes a made-up quarter" --body "The two-sample demo becomes a fictional roastery's first quarter: 18 prepared documents, a library home, a journey for every document and four review kinds. No key, API call, model, upload or storage."
gh pr checks --watch
gh pr merge --squash --delete-branch
git switch main
git pull
```

Expected: every check green, the PR squash-merged, `main` updated.

---

## Slice 2: the workspace and the review queue

Branch: `feat/demo-workspace`.

### Task 3: The demo workspace and review queue

**Files:**
- Modify: `crates/clearcut-server/src/web/app.js` (insert before the `// ---- bulk intake ----` marker; update `demoHome`; update `route`)
- Modify: `crates/clearcut-server/src/web/app.css` (slice 2 block before the `/* small */` marker)

**Interfaces:**
- Consumes: `demoDocs`, `demoResolve`, `demoMoney`, `demoStatedGross`, `demoManifest`, `demoStrip`, `badge`, `STATUS`.
- Produces: `demoWorkspace`, `demoQueue`, `demoStatusCounts`, and the workspace/queue routes.

- [ ] **Step 1: Write the failing test**

Add to `crates/clearcut-server/src/web.rs`:

```rust
    #[tokio::test]
    async fn the_demo_workspace_and_queue_match_the_quarter() {
        let (_, _, js) = fetch("/app.js").await;
        for marker in [
            "function demoWorkspace",
            "function demoQueue",
            "\"#/demo/workspace\"",
            "\"#/demo/review\"",
            "Open the workspace",
            "See what needs a person",
            "Nothing is waiting for a person.",
            "Review this document",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p clearcut-server web::tests::the_demo_workspace_and_queue_match_the_quarter`

Expected: FAIL.

- [ ] **Step 3: Add the status counts and the workspace**

Insert before `// ---------------------------------------------------------------- bulk intake ----`:

```js
function demoStatusCounts(docs) {
  return {
    all: docs.length,
    done: docs.filter((doc) => doc.status === "done").length,
    needs_review: docs.filter((doc) => doc.status === "needs_review").length,
    processing: docs.filter((doc) => doc.status === "processing").length,
  };
}

function demoWorkspace() {
  stopPolling();
  let filter = "all";
  const body = h("div", {});
  const draw = () => {
    const docs = demoDocs();
    const counts = demoStatusCounts(docs);
    const shown = filter === "all" ? docs : docs.filter((doc) => doc.status === filter);
    body.replaceChildren(
      h("div", { class: "stat-grid" },
        h("div", { class: "stat-card" }, h("b", {}, counts.all), h("span", {}, "documents")),
        h("div", { class: "stat-card" }, h("b", {}, counts.done), h("span", {}, "done")),
        h("div", { class: "stat-card" }, h("b", {}, counts.needs_review), h("span", {}, "need a person")),
        h("div", { class: "stat-card" }, h("b", {}, counts.processing), h("span", {}, "reading"))),
      h("p", { class: "small muted" }, "13 of the 17 finished documents never needed a person (76.5%)."),
      h("div", { class: "filters" },
        [["all", "All"], ["needs_review", "Needs review"], ["done", "Done"], ["processing", "Reading"]].map(([value, label]) =>
          h("button", {
            type: "button", class: "filter", "aria-pressed": String(filter === value),
            onclick: () => { filter = value; draw(); },
          }, label + " (" + counts[value] + ")"))),
      h("div", { class: "table-wrap" }, h("table", { class: "demo-table" },
        h("caption", { class: "small muted" }, "Documents in this demo"),
        h("thead", {}, h("tr", {},
          h("th", {}, "File"), h("th", {}, "Supplier"), h("th", {}, "Number"),
          h("th", {}, "Date"), h("th", {}, "Status"), h("th", { class: "num" }, "Gross"))),
        h("tbody", {}, shown.map((doc) => h("tr", {},
          h("td", { "data-label": "File" }, h("a", { href: "#/demo/document/" + doc.id }, doc.file)),
          h("td", { "data-label": "Supplier" }, doc.supplier,
            doc.status === "needs_review"
              ? h("span", { class: "held-reason" }, doc.check.label)
              : null),
          h("td", { "data-label": "Number" }, doc.number),
          h("td", { "data-label": "Date" }, doc.dateText),
          h("td", { "data-label": "Status" }, badge(doc.status)),
          h("td", { "data-label": "Gross", class: "num" }, demoMoney(demoStatedGross(doc), doc.currency))))))),
      h("p", { class: "small" },
        h("a", { href: "#/demo/review" }, "See what needs a person")));
  };
  show(h("div", { class: "page demo" },
    demoStrip(),
    h("div", { class: "page-head" },
      h("p", { class: "eyebrow" }, "Demo workspace"),
      h("h1", {}, "Documents"),
      h("p", { class: "lede" }, "The inbox after the roastery's first quarter. Confirming a document in this demo moves it from Needs review to Done here, without a request.")),
    body));
  draw();
}

function demoQueue() {
  stopPolling();
  const held = demoDocs().filter((doc) => doc.status === "needs_review");
  show(h("div", { class: "page demo" },
    demoStrip(),
    h("div", { class: "page-head" },
      h("p", { class: "eyebrow" }, "Demo workspace"),
      h("h1", {}, "Needs a person"),
      h("p", { class: "lede" }, "The documents the prepared reading could not settle. Each one says what a person has to decide.")),
    held.length === 0
      ? h("div", { class: "card" },
          h("h2", {}, "Nothing is waiting for a person."),
          h("p", {}, "Every document in this demo has been settled."),
          h("a", { class: "button", href: "#/demo/workspace" }, "Back to the workspace"))
      : h("div", { class: "queue" }, held.map((doc) =>
          h("div", { class: "queue-item" },
            h("strong", {}, doc.supplier),
            h("span", { class: "sample-file" }, doc.file),
            h("span", {}, doc.reason),
            h("span", { class: "small muted" }, "Gross " + demoMoney(demoStatedGross(doc), doc.currency)),
            h("a", { class: "button", href: "#/demo/document/" + doc.id }, "Review this document")))),
    h("p", { class: "small" }, h("a", { href: "#/demo" }, "← Back to the demo"))));
}
```

- [ ] **Step 4: Add the two home buttons**

In `demoHome`, after the fictional-roastery paragraph and before the closing `)),` of the page head, add:

```js
      h("div", { class: "demo-hero-actions" },
        h("a", { class: "button", href: "#/demo/workspace" }, "Open the workspace"),
        h("a", { class: "button secondary", href: "#/demo/review" }, "See what needs a person")),
```

- [ ] **Step 5: Add the two routes**

In `route()`, after `if (hash === "#/demo") return demoHome();`, add:

```js
  if (hash === "#/demo/workspace") return demoWorkspace();
  if (hash === "#/demo/review") return demoQueue();
```

- [ ] **Step 6: Add the slice 2 CSS**

Insert immediately before the `/* small */` marker, after the slice 1 rules:

```css
.stat-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 14px; margin: 18px 0; }
.stat-card { padding: 16px 18px; background: var(--surface); border: 1px solid var(--rule); border-radius: var(--radius); }
.stat-card b { display: block; font-family: var(--serif); font-size: 30px; line-height: 1.1; }
.stat-card span { color: var(--muted); font-size: 13px; }
.filters { display: flex; flex-wrap: wrap; gap: 8px; margin: 18px 0; }
.filter {
  font: 600 14px/1 var(--sans); padding: 9px 14px; border-radius: 999px;
  border: 1px solid var(--rule-strong); background: var(--surface); color: var(--ink);
  cursor: pointer; transition: background 120ms ease, border-color 120ms ease;
}
.filter[aria-pressed="true"] { background: var(--accent); color: var(--accent-ink); border-color: var(--accent); }
.held-reason { display: block; color: var(--muted); font-size: 12.5px; margin-top: 3px; }
.queue { display: grid; gap: 14px; }
.queue-item {
  display: grid; gap: 6px; padding: 20px;
  background: var(--surface); border: 1px solid var(--rule); border-radius: var(--radius);
}
@media (max-width: 720px) {
  .demo-table thead { display: none; }
  .demo-table tr { display: grid; grid-template-columns: 1fr 1fr; gap: 6px 12px; padding: 14px 0; border-bottom: 1px solid var(--rule); }
  .demo-table td { display: block; padding: 0; border: 0; }
  .demo-table td::before { content: attr(data-label); display: block; font-size: 11px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--muted); }
}
@media (max-width: 900px) {
  .stat-grid { grid-template-columns: 1fr 1fr; }
}
@media (max-width: 600px) {
  .stat-grid { grid-template-columns: 1fr; }
}
@media (prefers-reduced-motion: reduce) {
  .filter, .sample-card { transition: none; }
}
```

- [ ] **Step 7: Check and run**

```bash
node --check crates/clearcut-server/src/web/app.js
cargo test -p clearcut-server web::tests
```

Expected: both pass.

- [ ] **Step 8: Walk it in the browser**

At `#/demo/workspace`: 18 documents, 13 done, 4 need a person, 1 reading. The filters show 18, 4, 13, 1. Below 720 pixels the rows become cards with labels.

At `#/demo/review`: four entries with the four reasons. Confirm BST from the queue, then return to the workspace: done becomes 14, needs review 3, and BST is no longer in the queue. Confirm all four: the workspace shows 17 done, 0 awaiting, 1 reading; the queue says "Nothing is waiting for a person."

Reload: every confirmation is gone. "Start over" clears them without leaving the demo. "Exit demo" clears them and returns to `#/`.

- [ ] **Step 9: Commit**

```bash
git add crates/clearcut-server/src/web/app.js crates/clearcut-server/src/web/app.css crates/clearcut-server/src/web.rs
git commit -m "feat(web): the demo workspace and review queue"
```

### Task 4: Prove slice 2, document it, ship it

**Files:**
- Modify: `docs/LAUNCH.md:14`
- Modify: `docs/WORKLOG.md`

- [ ] **Step 1: Run the gate**

```bash
node --check crates/clearcut-server/src/web/app.js
cargo fmt --all
cargo clippy -p clearcut-server --all-targets -- -D warnings
cargo test -p clearcut-server
```

Expected: all pass.

- [ ] **Step 2: Update LAUNCH**

Replace the demo bullet's second sentence with:

```markdown
The workspace and the review queue are live in the demo; the register and batch replay follow in the next slices.
```

- [ ] **Step 3: Add a worklog line**

Add to the part 82 entry, under **Done**:

```markdown
- **A demo workspace and review queue.** 18 documents with four live counts, filters that update after a confirmation, held rows that carry their reason, and a queue that empties as the four documents are settled. All state stays in page memory; a reload, Start over or Exit demo clears it.
```

- [ ] **Step 4: Ship**

```bash
git add docs/LAUNCH.md docs/WORKLOG.md
git commit -m "docs: record the demo workspace"
git push -u origin feat/demo-workspace
gh pr create --title "feat(web): the demo workspace and review queue" --body "The demo gains its Documents list and review queue, with counts and filters that follow local confirmations. No request leaves the page."
gh pr checks --watch
gh pr merge --squash --delete-branch
git switch main
git pull
```

---

## Slice 3: the register and its five downloads

Branch: `feat/demo-register`.

### Task 5: The register model, screen, audit and downloads

**Files:**
- Modify: `crates/clearcut-server/src/web/app.js` (insert before the `// ---- bulk intake ----` marker; update `demoHome`; update `route`)
- Modify: `crates/clearcut-server/src/web/app.css`

**Interfaces:**
- Consumes: `demoDocs`, `demoResolve`, `demoDoc`, `demoMoney`, `demoStatedGross`, `demoCsvCell`, `STATUS`, `rateText`, `DEMO_*` constants.
- Produces: `demoRegister`, `demoStatedTotals`, `demoToCheck`, `demoManifest`, `demoAudit`, `demoRegisterPage`, `demoRegisterPreview`, `demoRegisterFile`, `demoRegisterName`, `demoDownload`, `demoMdCell`, `demoEscape`, and the `#/demo/register` route.

- [ ] **Step 1: Write the failing test**

Add to `crates/clearcut-server/src/web.rs`:

```rust
    #[tokio::test]
    async fn the_demo_register_says_what_it_is_and_downloads_five_files() {
        let (_, _, js) = fetch("/app.js").await;
        for marker in [
            "function demoRegister",
            "function demoRegisterPage",
            "function demoRegisterFile",
            "clearcut-demo-register-2026-03-01-to-2026-05-31",
            "ClearCut demo: sample data from a made-up roastery. Not a real register.",
            "Prepared data checked.",
            "\"Markdown\"",
            "\"Plain text\"",
            "VAT by category",
            "Stated totals",
            "Line arithmetic",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p clearcut-server web::tests::the_demo_register_says_what_it_is_and_downloads_five_files`

Expected: FAIL.

- [ ] **Step 3: Add the register model**

Insert before `// ---------------------------------------------------------------- bulk intake ----`:

```js
const DEMO_REGISTER_NOTICE = "ClearCut demo: sample data from a made-up roastery. Not a real register.";
const DEMO_REGISTER_FROM = "2026-03-01";
const DEMO_REGISTER_TO = "2026-05-31";

function demoStatedTotals(docs) {
  return docs
    .filter((doc) => doc.status === "done" || doc.status === "needs_review")
    .map((doc) => {
      const gross = demoStatedGross(doc);
      return {
        date: doc.dateText,
        supplier: doc.supplier,
        number: doc.number,
        currency: doc.currency,
        net: doc.net,
        vat: doc.vat,
        gross,
        agrees: doc.net + doc.vat === gross,
      };
    });
}

function demoToCheck(docs) {
  const rows = [];
  for (const doc of docs) {
    if (doc.status === "processing") {
      rows.push({ file: doc.file, reason: "Still reading", page: "–", sentence: "Uploaded minutes ago. Its figures are not in the register yet." });
    } else if (doc.status === "needs_review" && doc.check) {
      rows.push({
        file: doc.file,
        reason: doc.check.label,
        page: doc.check.page == null ? "–" : doc.check.page,
        sentence: doc.check.sentence,
      });
    }
  }
  return rows;
}

function demoManifest(docs) {
  const done = docs.filter((doc) => doc.status === "done").length;
  const awaiting = docs.filter((doc) => doc.status === "needs_review").length;
  const reading = docs.filter((doc) => doc.status === "processing").length;
  const finished = done + awaiting;
  return {
    period: DEMO_PERIOD,
    documents: docs.length,
    done,
    awaiting,
    reading,
    finished,
    without_person: docs.filter((doc) => doc.status === "done" && !doc.held).length,
    settled: docs.filter((doc) => doc.confirmed).length,
    automation_bps: DEMO_AUTOMATION_BPS,
    undated: docs.filter((doc) => doc.status !== "processing" && !doc.date).length,
    currencies: [...new Set(docs.filter((doc) => doc.status !== "processing").map((doc) => doc.currency))],
    credit_notes: docs.filter((doc) => doc.kind === "credit_note" && doc.status !== "processing").length,
    notice: DEMO_REGISTER_NOTICE,
  };
}

function demoAudit(docs) {
  const issues = [];
  const expected = new Set();
  for (const doc of docs) {
    if (doc.status === "processing") continue;
    let net = 0;
    let vat = 0;
    for (const line of doc.lines) {
      if (line.net + line.vat !== line.gross) {
        issues.push(doc.file + ": a line's net plus VAT is not its gross.");
      }
      const arithmetic = line.inclusive ? line.gross : line.net;
      if (line.quantity * line.unit !== arithmetic) {
        if (doc.id === "pp-0091" && line.description === "Compostable bowls") {
          expected.add("PP-0091 line 37 is one cent below its arithmetic");
        } else {
          issues.push(doc.file + ": a line's quantity times unit price is not its amount.");
        }
      }
      net += line.net;
      vat += line.vat;
    }
    if (net !== doc.net || vat !== doc.vat) {
      issues.push(doc.file + ": the lines do not sum to the document's net and VAT.");
    }
    if (doc.net + doc.vat !== doc.gross) {
      issues.push(doc.file + ": net plus VAT is not the gross.");
    }
    if (doc.status === "needs_review" && doc.held === "figure" && doc.grossRead !== doc.grossPrinted) {
      expected.add("BST-221's reading is one rand above the printed total");
    }
  }
  return { issues, expected: [...expected] };
}

function demoRegister() {
  const docs = demoDocs();
  const finished = docs.filter((doc) => doc.status === "done" || doc.status === "needs_review");
  const rows = finished
    .map((doc) => ({
      id: doc.id,
      date: doc.date,
      dateText: doc.dateText,
      supplier: doc.supplier,
      number: doc.number,
      kind: DEMO_KIND[doc.kind],
      status: doc.status,
      currency: doc.currency,
      net: doc.net,
      vat: doc.vat,
      gross: demoStatedGross(doc),
      lines: doc.lines.length,
    }))
    .sort((a, b) => (a.date || "9999-99-99").localeCompare(b.date || "9999-99-99") || a.supplier.localeCompare(b.supplier));

  const groups = new Map();
  for (const doc of finished) {
    if (!doc.date) continue;
    const month = doc.date.slice(0, 7);
    const byTax = new Map();
    for (const line of doc.lines) {
      const found = byTax.get(line.tax) || { taxable: 0, vat: 0, docs: new Set() };
      found.taxable += line.net;
      found.vat += line.vat;
      found.docs.add(doc.id);
      byTax.set(line.tax, found);
    }
    for (const [tax, sums] of byTax) {
      const side = doc.status === "done" ? "Ready" : "Awaiting review";
      const key = [month, doc.currency, doc.kind, tax, side].join("|");
      const group = groups.get(key) || {
        month,
        currency: doc.currency,
        kind: DEMO_KIND[doc.kind],
        kindOrder: DEMO_KIND_ORDER[doc.kind],
        category: DEMO_TAX[tax],
        taxOrder: DEMO_TAX_ORDER[tax],
        rate: DEMO_RATE[tax],
        side,
        documents: new Set(),
        taxable: 0,
        vat: 0,
      };
      group.taxable += sums.taxable;
      group.vat += sums.vat;
      for (const id of sums.docs) group.documents.add(id);
      groups.set(key, group);
    }
  }
  const vatByCategory = [...groups.values()]
    .map((group) => ({ ...group, documents: group.documents.size }))
    .sort((a, b) =>
      a.month.localeCompare(b.month) ||
      a.currency.localeCompare(b.currency) ||
      a.kindOrder - b.kindOrder ||
      a.taxOrder - b.taxOrder ||
      (a.side === "Ready" ? 0 : 1) - (b.side === "Ready" ? 0 : 1));

  const stated = demoStatedTotals(docs);
  return {
    rows,
    vatByCategory,
    stated,
    toCheck: demoToCheck(docs),
    manifest: demoManifest(docs),
    audit: demoAudit(docs),
    vatAgrees: rows
      .filter((row) => row.date)
      .every((row) => {
        const doc = demoDoc(row.id);
        return doc.lines.reduce((sum, line) => sum + line.vat, 0) === doc.vat;
      }),
  };
}
```

- [ ] **Step 4: Add the register screen**

Insert after `demoRegister`:

```js
function demoRegisterPreview(model) {
  const m = model.manifest;
  const parts = [];

  parts.push(h("div", { class: "card" },
    h("h2", {}, "This register"),
    h("p", {}, "Period fixed in the demo: " + DEMO_PERIOD + "."),
    h("p", {}, m.documents + " documents: " + m.done + " done, " + m.awaiting + " awaiting review, " + m.reading + " still reading."),
    h("p", {}, rateText(m.automation_bps) + " of finished documents needed nobody: " + m.without_person + " of " + m.finished + "."),
    h("p", { class: "small muted" }, "Credit notes are kept apart and currencies are never added together.")));

  parts.push(h("h2", {}, "Documents"));
  parts.push(h("div", { class: "table-wrap" }, h("table", { class: "demo-table" },
    h("caption", { class: "small muted" }, "The " + model.rows.length + " finished documents"),
    h("thead", {}, h("tr", {},
      h("th", {}, "Date"), h("th", {}, "Supplier"), h("th", {}, "Number"), h("th", {}, "Kind"),
      h("th", {}, "Status"), h("th", {}, "Currency"), h("th", { class: "num" }, "Net"),
      h("th", { class: "num" }, "VAT"), h("th", { class: "num" }, "Gross"), h("th", { class: "num" }, "Lines"))),
    h("tbody", {}, model.rows.map((row) => h("tr", {},
      h("td", { "data-label": "Date" }, row.dateText),
      h("td", { "data-label": "Supplier" }, row.supplier),
      h("td", { "data-label": "Number" }, row.number),
      h("td", { "data-label": "Kind" }, row.kind),
      h("td", { "data-label": "Status" }, badge(row.status)),
      h("td", { "data-label": "Currency" }, row.currency),
      h("td", { "data-label": "Net", class: "num" }, demoMoney(row.net, row.currency)),
      h("td", { "data-label": "VAT", class: "num" }, demoMoney(row.vat, row.currency)),
      h("td", { "data-label": "Gross", class: "num" }, demoMoney(row.gross, row.currency)),
      h("td", { "data-label": "Lines", class: "num" }, row.lines)))))));

  parts.push(h("h2", {}, "VAT by category"));
  parts.push(h("div", { class: "table-wrap" }, h("table", { class: "demo-table" },
    h("caption", { class: "small muted" }, "Dated documents, grouped by month, currency, kind, category, rate and side"),
    h("thead", {}, h("tr", {},
      h("th", {}, "Month"), h("th", {}, "Currency"), h("th", {}, "Kind"), h("th", {}, "Category"),
      h("th", { class: "num" }, "Rate"), h("th", {}, "Side"), h("th", { class: "num" }, "Documents"),
      h("th", { class: "num" }, "Taxable"), h("th", { class: "num" }, "VAT"))),
    h("tbody", {}, model.vatByCategory.map((group) => h("tr", {},
      h("td", { "data-label": "Month" }, group.month),
      h("td", { "data-label": "Currency" }, group.currency),
      h("td", { "data-label": "Kind" }, group.kind),
      h("td", { "data-label": "Category" }, group.category),
      h("td", { "data-label": "Rate", class: "num" }, rateText(group.rate)),
      h("td", { "data-label": "Side" }, group.side),
      h("td", { "data-label": "Documents", class: "num" }, group.documents),
      h("td", { "data-label": "Taxable", class: "num" }, demoMoney(group.taxable, group.currency)),
      h("td", { "data-label": "VAT", class: "num" }, demoMoney(group.vat, group.currency))))))));

  parts.push(h("h2", {}, "Stated totals"));
  parts.push(h("div", { class: "table-wrap" }, h("table", { class: "demo-table" },
    h("caption", { class: "small muted" }, "Each finished document's own totals"),
    h("thead", {}, h("tr", {},
      h("th", {}, "Date"), h("th", {}, "Supplier"), h("th", {}, "Number"),
      h("th", { class: "num" }, "Net"), h("th", { class: "num" }, "VAT"),
      h("th", { class: "num" }, "Gross"), h("th", {}, "Net plus VAT"))),
    h("tbody", {}, model.stated.map((row) => h("tr", {},
      h("td", { "data-label": "Date" }, row.date),
      h("td", { "data-label": "Supplier" }, row.supplier),
      h("td", { "data-label": "Number" }, row.number),
      h("td", { "data-label": "Net", class: "num" }, demoMoney(row.net, row.currency)),
      h("td", { "data-label": "VAT", class: "num" }, demoMoney(row.vat, row.currency)),
      h("td", { "data-label": "Gross", class: "num" }, demoMoney(row.gross, row.currency)),
      h("td", { "data-label": "Net plus VAT" }, row.agrees ? "Agrees" : "Differs by " + demoMoney(row.gross - row.net - row.vat, row.currency))))))));
  parts.push(h("p", { class: "small muted" }, model.vatAgrees
    ? "Every dated document's stated VAT agrees with its lines."
    : "A dated document's stated VAT does not agree with its lines. See the audit below."));

  parts.push(h("h2", {}, "To check"));
  if (model.toCheck.length === 0) {
    parts.push(h("p", { class: "small muted" }, "Nothing is waiting for a person."));
  } else {
    parts.push(h("div", { class: "table-wrap" }, h("table", { class: "demo-table" },
      h("caption", { class: "small muted" }, "Reasons this register is not final"),
      h("thead", {}, h("tr", {},
        h("th", {}, "Document"), h("th", {}, "Reason"), h("th", { class: "num" }, "Page"), h("th", {}, "What to check"))),
      h("tbody", {}, model.toCheck.map((row) => h("tr", {},
        h("td", { "data-label": "Document" }, row.file),
        h("td", { "data-label": "Reason" }, row.reason),
        h("td", { "data-label": "Page", class: "num" }, row.page),
        h("td", { "data-label": "What to check" }, row.sentence)))))));
  }

  parts.push(h("h2", {}, "Manifest"));
  const m2 = model.manifest;
  const manifestRows = [
    ["Period", m2.period],
    ["Documents", m2.documents],
    ["Done", m2.done],
    ["Awaiting review", m2.awaiting],
    ["Still reading", m2.reading],
    ["Finished", m2.finished],
    ["Finished without a person", m2.without_person],
    ["Settled by a person", m2.settled],
    ["Automation rate", rateText(m2.automation_bps)],
    ["Undated", m2.undated],
    ["Currencies", m2.currencies.join(", ")],
    ["Credit notes", m2.credit_notes],
    ["Notice", m2.notice],
  ];
  parts.push(h("div", { class: "table-wrap" }, h("table", { class: "demo-table" },
    h("caption", { class: "small muted" }, "What this register counted"),
    h("tbody", {}, manifestRows.map(([label, value]) => h("tr", {},
      h("th", { scope: "row" }, label),
      h("td", {}, value)))))));

  parts.push(h("p", { class: model.audit.issues.length ? "small error" : "small audit-ok", role: "status" },
    model.audit.issues.length
      ? "The prepared data does not check out: " + model.audit.issues.join(" ")
      : "Prepared data checked."));
  if (model.audit.expected.length) {
    parts.push(h("p", { class: "small muted" }, "Expected in this sample: " + model.audit.expected.join("; ") + "."));
  }
  return parts;
}

function demoRegisterName(ext) {
  return "clearcut-demo-register-2026-03-01-to-2026-05-31." + ext;
}

function demoDownload(name, text, type) {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const link = h("a", { href: url, download: name });
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10000);
}

function demoMdCell(value) {
  return String(value == null ? "" : value).replaceAll("|", "\\|").replaceAll("\n", " ");
}

function demoEscape(value) {
  return String(value == null ? "" : value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function demoRegisterFile(model, format) {
  const m = model.manifest;
  if (format === "json") {
    return JSON.stringify({
      notice: DEMO_REGISTER_NOTICE,
      period: { from: DEMO_REGISTER_FROM, to: DEMO_REGISTER_TO, label: DEMO_PERIOD },
      manifest: m,
      rows: model.rows.map((row) => ({
        date: row.date,
        supplier: row.supplier,
        number: row.number,
        kind: row.kind,
        status: row.status,
        currency: row.currency,
        net: row.net,
        vat: row.vat,
        gross: row.gross,
        line_count: row.lines,
      })),
      vat_by_category: model.vatByCategory.map((group) => ({
        month: group.month,
        currency: group.currency,
        kind: group.kind,
        category: group.category,
        rate_bps: group.rate,
        side: group.side,
        documents: group.documents,
        taxable: group.taxable,
        vat: group.vat,
      })),
      stated_totals: model.stated.map((row) => ({
        date: row.date,
        supplier: row.supplier,
        number: row.number,
        currency: row.currency,
        net: row.net,
        vat: row.vat,
        gross: row.gross,
        net_plus_vat_is_gross: row.agrees,
      })),
      to_check: model.toCheck,
      audit: model.audit,
    }, null, 2) + "\n";
  }

  if (format === "csv") {
    const rows = [[demoCsvCell(DEMO_REGISTER_NOTICE)]];
    rows.push(["Date", "Supplier", "Number", "Kind", "Status", "Currency", "Net", "VAT", "Gross", "Lines"].map(demoCsvCell).join(","));
    for (const row of model.rows) {
      rows.push([
        row.dateText, row.supplier, row.number, row.kind, STATUS[row.status] || row.status,
        row.currency, demoMoney(row.net, row.currency), demoMoney(row.vat, row.currency),
        demoMoney(row.gross, row.currency), row.lines,
      ].map(demoCsvCell).join(","));
    }
    return rows.join("\n") + "\n";
  }

  if (format === "md") {
    const lines = [
      DEMO_REGISTER_NOTICE,
      "",
      "# Register",
      "",
      "Period: " + DEMO_PERIOD,
      "",
      m.documents + " documents: " + m.done + " done, " + m.awaiting + " awaiting review, " + m.reading + " still reading. " + rateText(m.automation_bps) + " of finished documents needed nobody.",
      "",
      "## Documents",
      "",
      "| Date | Supplier | Number | Kind | Status | Currency | Net | VAT | Gross | Lines |",
      "|---|---|---|---|---|---|---:|---:|---:|---:|",
      ...model.rows.map((row) => "| " + [
        row.dateText, row.supplier, row.number, row.kind, STATUS[row.status] || row.status,
        row.currency, demoMoney(row.net, row.currency), demoMoney(row.vat, row.currency),
        demoMoney(row.gross, row.currency), row.lines,
      ].map(demoMdCell).join(" | ") + " |"),
      "",
      "## VAT by category",
      "",
      "| Month | Currency | Kind | Category | Rate | Side | Documents | Taxable | VAT |",
      "|---|---|---|---|---:|---|---:|---:|---:|",
      ...model.vatByCategory.map((group) => "| " + [
        group.month, group.currency, group.kind, group.category, rateText(group.rate), group.side,
        group.documents, demoMoney(group.taxable, group.currency), demoMoney(group.vat, group.currency),
      ].map(demoMdCell).join(" | ") + " |"),
      "",
      "## To check",
      "",
      model.toCheck.length
        ? model.toCheck.map((row) => "- " + row.file + ": " + row.reason + ". " + row.sentence).join("\n")
        : "Nothing is waiting for a person.",
    ];
    return lines.join("\n") + "\n";
  }

  if (format === "txt") {
    const lines = [
      DEMO_REGISTER_NOTICE,
      "",
      "REGISTER",
      "Period: " + DEMO_PERIOD,
      m.documents + " documents: " + m.done + " done, " + m.awaiting + " awaiting review, " + m.reading + " still reading.",
      "",
      "DOCUMENTS",
      ["Date", "Supplier", "Number", "Kind", "Status", "Currency", "Net", "VAT", "Gross", "Lines"].join(" | "),
      ...model.rows.map((row) => [
        row.dateText, row.supplier, row.number, row.kind, STATUS[row.status] || row.status,
        row.currency, demoMoney(row.net, row.currency), demoMoney(row.vat, row.currency),
        demoMoney(row.gross, row.currency), row.lines,
      ].join(" | ")),
      "",
      "VAT BY CATEGORY",
      ["Month", "Currency", "Kind", "Category", "Rate", "Side", "Documents", "Taxable", "VAT"].join(" | "),
      ...model.vatByCategory.map((group) => [
        group.month, group.currency, group.kind, group.category, rateText(group.rate), group.side,
        group.documents, demoMoney(group.taxable, group.currency), demoMoney(group.vat, group.currency),
      ].join(" | ")),
      "",
      "TO CHECK",
      model.toCheck.length
        ? model.toCheck.map((row) => row.file + ": " + row.reason + ". " + row.sentence).join("\n")
        : "Nothing is waiting for a person.",
    ];
    return lines.join("\n") + "\n";
  }

  const rows = model.rows.map((row) => "<tr>" + [
    row.dateText, row.supplier, row.number, row.kind, STATUS[row.status] || row.status,
    row.currency, demoMoney(row.net, row.currency), demoMoney(row.vat, row.currency),
    demoMoney(row.gross, row.currency), row.lines,
  ].map((cell) => "<td>" + demoEscape(cell) + "</td>").join("") + "</tr>").join("\n");
  const groups = model.vatByCategory.map((group) => "<tr>" + [
    group.month, group.currency, group.kind, group.category, rateText(group.rate), group.side,
    group.documents, demoMoney(group.taxable, group.currency), demoMoney(group.vat, group.currency),
  ].map((cell) => "<td>" + demoEscape(cell) + "</td>").join("") + "</tr>").join("\n");
  const checks = model.toCheck.length
    ? "<ul>" + model.toCheck.map((row) => "<li>" + demoEscape(row.file + ": " + row.reason + ". " + row.sentence) + "</li>").join("") + "</ul>"
    : "<p>Nothing is waiting for a person.</p>";
  return [
    "<!doctype html>",
    "<!-- " + DEMO_REGISTER_NOTICE + " -->",
    "<html lang=\"en\"><head><meta charset=\"utf-8\"><title>ClearCut demo register</title>",
    "<style>body{font:15px/1.5 system-ui,sans-serif;margin:32px;color:#17191a}h1{font-size:26px}table{border-collapse:collapse;margin:12px 0 28px}th,td{border:1px solid #ccc;padding:6px 9px;text-align:left}th{background:#f3f1ea}.notice{background:#fff4d6;border:1px solid #c99a2e;padding:10px 12px}</style>",
    "</head><body>",
    "<p class=\"notice\">" + demoEscape(DEMO_REGISTER_NOTICE) + "</p>",
    "<h1>Register</h1>",
    "<p>Period: " + demoEscape(DEMO_PERIOD) + "</p>",
    "<p>" + demoEscape(m.documents + " documents: " + m.done + " done, " + m.awaiting + " awaiting review, " + m.reading + " still reading. " + rateText(m.automation_bps) + " of finished documents needed nobody.") + "</p>",
    "<h2>Documents</h2>",
    "<table><thead><tr><th>Date</th><th>Supplier</th><th>Number</th><th>Kind</th><th>Status</th><th>Currency</th><th>Net</th><th>VAT</th><th>Gross</th><th>Lines</th></tr></thead><tbody>",
    rows,
    "</tbody></table>",
    "<h2>VAT by category</h2>",
    "<table><thead><tr><th>Month</th><th>Currency</th><th>Kind</th><th>Category</th><th>Rate</th><th>Side</th><th>Documents</th><th>Taxable</th><th>VAT</th></tr></thead><tbody>",
    groups,
    "</tbody></table>",
    "<h2>To check</h2>",
    checks,
    "</body></html>",
  ].join("\n") + "\n";
}

function demoRegisterPage() {
  stopPolling();
  const model = demoRegister();
  const status = h("p", { class: "small muted", role: "status" }, "");
  const buttons = h("div", { class: "downloads" },
    [["json", "JSON", "application/json"], ["csv", "CSV", "text/csv"], ["md", "Markdown", "text/markdown"],
      ["txt", "Plain text", "text/plain"], ["html", "HTML", "text/html"]].map(([ext, label, type]) =>
      h("button", {
        type: "button", class: "secondary",
        onclick: () => {
          demoDownload(demoRegisterName(ext), demoRegisterFile(model, ext), type);
          status.textContent = "Built in this page. Nothing was requested from a server.";
        },
      }, label)));
  show(h("div", { class: "page demo" },
    demoStrip(),
    h("div", { class: "page-head" },
      h("p", { class: "eyebrow" }, "Demo register"),
      h("h1", {}, "Register"),
      h("p", { class: "lede" }, "Every finished document in the quarter, with its figures, its VAT and the things that need a person. The period is fixed in the demo: " + DEMO_PERIOD + ".")),
    demoRegisterPreview(model),
    h("div", { class: "card" },
      h("h2", {}, "Downloads"),
      h("p", { class: "small muted" }, "The real app also builds Excel, a zipped CSV bundle and PDF. The demo builds these five in the browser."),
      buttons,
      status),
    h("p", { class: "small" }, h("a", { href: "#/demo" }, "← Back to the demo"))));
}
```

- [ ] **Step 5: Add the register link to the home and the route**

In `demoHome`, append inside `.demo-hero-actions`:

```js
        h("a", { class: "button secondary", href: "#/demo/register" }, "Open the register"),
```

In `route()`, after the review route, add:

```js
  if (hash === "#/demo/register") return demoRegisterPage();
```

- [ ] **Step 6: Add the slice 3 CSS**

Insert immediately before the `/* small */` marker:

```css
.downloads { display: flex; flex-wrap: wrap; gap: 10px; margin-top: 12px; }
.audit-ok { color: var(--done); }
.demo-table caption { text-align: left; padding: 10px 16px; border-bottom: 1px solid var(--rule); }
.demo-table th[scope="row"] { width: 38%; text-transform: none; letter-spacing: 0; font-size: 13.5px; color: var(--ink-soft); background: var(--surface); }
```

- [ ] **Step 7: Check and run**

```bash
node --check crates/clearcut-server/src/web/app.js
cargo test -p clearcut-server web::tests
```

Expected: both pass.

- [ ] **Step 8: Walk the register in the browser**

At `#/demo/register` before any confirmation:

- This register says 18 documents, 13 done, 4 awaiting, 1 reading, 76.5%.
- Documents has 17 rows; BST shows R 12 501.00; CS shows "No date".
- VAT by category has the nine rows in the spec's order and numbers, including March standard R 39 619.00 / R 5 942.85 and May awaiting R 48 216.49 / R 7 232.47.
- Stated totals shows BST differing by R 1.00 and every other row agreeing.
- To check has BST, NGC, PP, CS and RS-2215, in that order.
- "Prepared data checked." is shown, with the two expected issues named.
- Each download has the right name; the JSON matches the preview; the CSV's first row is the notice; the HTML opens as a page.

Then confirm all four from the queue and return: BST shows R 12 500.00 and agrees; CS is dated 12 May 2026 and sits in May; the May standard ready group has 5 documents, R 69 734.32 and R 10 460.14; To check has only RS-2215; the automation rate stays 76.5%; the manifest says 4 settled by a person.

- [ ] **Step 9: Commit**

```bash
git add crates/clearcut-server/src/web/app.js crates/clearcut-server/src/web/app.css crates/clearcut-server/src/web.rs
git commit -m "feat(web): the demo register and its five downloads"
```

### Task 6: Prove slice 3, document it, ship it

**Files:**
- Modify: `docs/LAUNCH.md:14`
- Modify: `docs/WORKLOG.md`

- [ ] **Step 1: Run the gate**

```bash
node --check crates/clearcut-server/src/web/app.js
cargo fmt --all
cargo clippy -p clearcut-server --all-targets -- -D warnings
cargo test -p clearcut-server
```

- [ ] **Step 2: Update LAUNCH**

The demo bullet now reads:

```markdown
- **A public demo** at `#/demo`: a made-up roastery's first quarter, 18 prepared supplier documents from March to May 2026, with a library home, a Documents list, a review queue, a full journey for every document, and a period register with five in-page downloads (JSON, CSV, Markdown, plain text and HTML). A batch replay follows in the last slice. It needs no key, calls no model, stores nothing and cannot reach a tenant; accounts remain invite-only.
```

- [ ] **Step 3: Add a worklog line**

Add under **Done** in the part 82 entry:

```markdown
- **A demo register.** Summed only from prepared line values: 17 finished rows, nine VAT groups, per-document stated totals, the exceptions, the manifest and a prepared-data audit that names the two expected issues. Five files are built in the page (JSON, CSV, Markdown, plain text, HTML) with a notice, the CSV formula-leader rule and escaped HTML. Confirming the four held documents moves BST's total by R 1.00, puts CS in May and leaves the automation rate at 76.5%.
```

- [ ] **Step 4: Ship**

```bash
git add docs/LAUNCH.md docs/WORKLOG.md
git commit -m "docs: record the demo register"
git push -u origin feat/demo-register
gh pr create --title "feat(web): the demo register and its five downloads" --body "The demo gains the register: 17 rows, VAT groups, stated totals, exceptions, the manifest, an audit and five files built in the page. No request leaves the page."
gh pr checks --watch
gh pr merge --squash --delete-branch
git switch main
git pull
```

---

## Slice 4: the batch replay, the cross-screen proof and the live check

Branch: `feat/demo-batch`.

### Task 7: The batch replay

**Files:**
- Modify: `crates/clearcut-server/src/web/app.js` (insert before the `// ---- bulk intake ----` marker; update `demoHome` and `demoWorkspace`; update `route`)
- Modify: `crates/clearcut-server/src/web/app.css`

**Interfaces:**
- Consumes: `demoStrip`, `demoEndCard`, `stopPolling`, `timer`.
- Produces: `demoBulk`, the `#/demo/bulk` route, the home button and the workspace link.

- [ ] **Step 1: Write the failing test**

Add to `crates/clearcut-server/src/web.rs`:

```rust
    #[tokio::test]
    async fn the_demo_batch_replay_tells_three_outcomes() {
        let (_, _, js) = fetch("/app.js").await;
        for marker in [
            "function demoBulk",
            "\"#/demo/bulk\"",
            "Replay the batch",
            "roastery-spares-rs-2215.pdf",
            "somerset-soc-114.pdf",
            "notes.txt",
            "No upload, no model call and no charge.",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p clearcut-server web::tests::the_demo_batch_replay_tells_three_outcomes`

Expected: FAIL.

- [ ] **Step 3: Add the batch replay**

Insert before `// ---------------------------------------------------------------- bulk intake ----`:

```js
function demoBulk() {
  stopPolling();
  const files = [
    { file: "roastery-spares-rs-2215.pdf", state: "Waiting", outcome: "", link: null },
    { file: "somerset-soc-114.pdf", state: "Waiting", outcome: "", link: null },
    { file: "notes.txt", state: "Waiting", outcome: "", link: null },
  ];
  const list = h("div", { class: "queue" });
  const status = h("p", { class: "small muted", role: "status" }, "Ready to replay.");
  const draw = () => {
    list.replaceChildren(...files.map((row) => h("div", { class: "queue-item batch-row" },
      h("strong", {}, row.file),
      h("span", { class: "batch-state" }, row.state),
      row.outcome ? h("span", { class: "small" }, row.outcome) : null,
      row.link ? h("a", { class: "small", href: row.link }, "Open the document") : null)));
  };
  const steps = [
    [0, 0, "Uploading"],
    [700, 0, "Reading"],
    [1500, 0, "Done"],
    [1900, 1, "Uploading"],
    [2500, 1, "Done"],
    [2900, 2, "Uploading"],
    [3500, 2, "Refused"],
  ];
  const outcomes = {
    0: "Accepted as RS-2215. It is the document already reading at the top of the workspace.",
    1: "The server returned SOC-114. No second read happened and no new document was made.",
    2: "notes.txt is not a PDF, PNG or JPEG, so it was refused.",
  };
  let at = 0;
  const advance = () => {
    if (at === steps.length) {
      status.textContent = "The replay is done. No upload, no model call and no charge.";
      return;
    }
    const [delay, index, state] = steps[at];
    timer = setTimeout(() => {
      files[index].state = state;
      if (outcomes[index] && (state === "Done" || state === "Refused")) files[index].outcome = outcomes[index];
      if (index === 0 && state === "Done") files[0].link = "#/demo/document/rs-2215";
      draw();
      at += 1;
      advance();
    }, delay);
  };
  const start = () => {
    stopPolling();
    for (const row of files) {
      row.state = "Waiting";
      row.outcome = "";
      row.link = null;
    }
    status.textContent = "Replaying the batch.";
    draw();
    at = 0;
    advance();
  };
  draw();
  show(h("div", { class: "page demo" },
    demoStrip(),
    h("div", { class: "page-head" },
      h("p", { class: "eyebrow" }, "Demo batch"),
      h("h1", {}, "Replay a batch"),
      h("p", { class: "lede" }, "Three files that show how a batch behaves: one new document, one duplicate, and one file that is refused. No upload, no model call and no charge.")),
    h("div", { class: "card" },
      h("p", { class: "small muted" }, "In the real app this screen sends each file to be read, one after another. Here the rows are simulated and the timers stop when you leave."),
      h("div", { class: "row" }, h("button", { type: "button", onclick: start }, "Replay the batch")),
      list,
      status),
    h("p", { class: "small" }, h("a", { href: "#/demo" }, "← Back to the demo"))));
  start();
}
```

- [ ] **Step 4: Add the batch link to the home and the workspace, and the route**

In `demoHome`, append inside `.demo-hero-actions`:

```js
        h("a", { class: "button secondary", href: "#/demo/bulk" }, "Replay a batch"),
```

In `demoWorkspace`, replace the single queue link paragraph with:

```js
      h("p", { class: "small" },
        h("a", { href: "#/demo/review" }, "See what needs a person"),
        " · ",
        h("a", { href: "#/demo/register" }, "Open the register"),
        " · ",
        h("a", { href: "#/demo/bulk" }, "Replay a batch")));
```

In `route()`, after the register route, add:

```js
  if (hash === "#/demo/bulk") return demoBulk();
```

- [ ] **Step 5: Add the slice 4 CSS**

Insert immediately before the `/* small */` marker:

```css
.batch-row { grid-template-columns: minmax(0, 1fr) auto; align-items: baseline; }
.batch-row strong { grid-column: 1; }
.batch-state { grid-column: 2; justify-self: end; font-size: 13px; color: var(--muted); }
.batch-row .small, .batch-row a { grid-column: 1 / -1; }
```

- [ ] **Step 6: Check and run**

```bash
node --check crates/clearcut-server/src/web/app.js
cargo test -p clearcut-server web::tests
```

- [ ] **Step 7: Walk the batch**

At `#/demo/bulk`: the three rows move through waiting, uploading, reading and their outcome; RS-2215 links to its reading document; SOC says the server returned it and no new document was made; notes.txt is refused. "Replay the batch" resets and runs again. Leaving the screen stops the timers (navigate away mid-run and back; the old run does not continue).

- [ ] **Step 8: Commit**

```bash
git add crates/clearcut-server/src/web/app.js crates/clearcut-server/src/web/app.css crates/clearcut-server/src/web.rs
git commit -m "feat(web): the demo batch replay"
```

### Task 8: Cross-screen proof, docs, gate, ship and live check

**Files:**
- Modify: `crates/clearcut-server/src/web.rs` (final marker test)
- Modify: `docs/LAUNCH.md`
- Modify: `docs/WORKLOG.md`

- [ ] **Step 1: Add the final route and copy test**

Add to `crates/clearcut-server/src/web.rs`:

```rust
    #[tokio::test]
    async fn the_demo_has_every_route_and_control() {
        let (_, _, js) = fetch("/app.js").await;
        for marker in [
            "Open the workspace",
            "See what needs a person",
            "Open the register",
            "Replay the batch",
            "\"#/demo/workspace\"",
            "\"#/demo/review\"",
            "\"#/demo/register\"",
            "\"#/demo/bulk\"",
            "#/demo/invoice/",
            "#/demo/document/",
            "demoJourney(legacy[1], true)",
            "demoJourney(journey[2], journey[1] === \"invoice\")",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
        let (_, _, css) = fetch("/app.css").await;
        for marker in [".stat-grid", ".queue-item", ".downloads", "@media (max-width: 720px)"] {
            assert!(css.contains(marker), "{marker} in app.css");
        }
    }
```

- [ ] **Step 2: Run every check**

```bash
node --check crates/clearcut-server/src/web/app.js
cargo fmt --all
cargo clippy -p clearcut-server --all-targets -- -D warnings
cargo test -p clearcut-server
```

Expected: all pass.

- [ ] **Step 3: Walk the whole demo once more, signed out**

With the network panel open:

- Every demo route issues zero `/v1` requests and no `Authorization` header.
- The workspace counts, filters, queue, register rows, VAT table and To check match the spec.
- Confirm all four from the queue: the workspace shows 17 done, 0 awaiting, 1 reading; BST's total corrects by R 1.00; CS-441 moves into May; the automation rate stays 76.5%.
- Each of the five downloads has the right name, first line and content; the JSON matches the preview.
- The batch replay accepts the reading document already in the list, makes no second document for the duplicate, refuses the text file, makes no upload, and stops its timers on leave.
- A reload loses every confirmation; Start over and Exit demo both clear the state.
- 1280 and 390 wide, light and dark, reduced motion, keyboard only. The workspace and register rows become cards below 720 pixels; focus moves to the confirmation banner; the filters read `aria-pressed`.

- [ ] **Step 4: Update LAUNCH**

Set the demo bullet to:

```markdown
- **A public demo** at `#/demo`: a made-up roastery's first quarter, 18 prepared supplier documents from March to May 2026, with a library home, a Documents list, a review queue, a full journey for every document, a period register with five in-page downloads (JSON, CSV, Markdown, plain text and HTML), and a batch replay. It needs no key, calls no model, stores nothing and cannot reach a tenant; accounts remain invite-only.
```

- [ ] **Step 5: Finish the worklog entry**

Add under **Done**, and replace the **Not done** line:

```markdown
- **A batch replay.** Three files, three outcomes: RS-2215 is accepted and reading, SOC-114 returns as the document already in the list, notes.txt is refused. Timers run on the existing single timer, so leaving the screen stops them; Replay resets and runs again.
- **Cross-screen proof.** Confirmations flow from the queue to the workspace and the register; BST corrects by R 1.00, CS lands in May, the automation rate stays 76.5%; the JSON download matches the preview; the demo is walked signed out with the network panel open and makes no `/v1` request.
- **Shipped and deployed.** Each slice squash-merged; the VM pulled `main` and rebuilt the server image; `/health` answered, `/` returned 200, and the served `app.js` carries `demoWorkspace`, `demoRegisterPage` and `demoBulk`.
**Not done:** the demo remains prepared data: it is not evidence of the reader's accuracy, and no visitor's own file is accepted.
```

- [ ] **Step 6: Run the gate and ship**

```bash
git add crates/clearcut-server/src/web.rs docs/LAUNCH.md docs/WORKLOG.md
git commit -m "docs: record the finished demo and its live check"
git push -u origin feat/demo-batch
gh pr create --title "feat(web): the demo batch replay and the finished quarter" --body "The last slice: the batch replay, the cross-screen tests, the docs and the live check. The demo now covers the inbox, the queue, the register and a batch."
gh pr checks --watch
gh pr merge --squash --delete-branch
git switch main
git pull
```

- [ ] **Step 7: Deploy and check the live host**

Follow `docs/DEPLOY-ORACLE.md` with the operator's existing SSH access. Do not print key material.

```bash
ssh ubuntu@145.241.100.85 'cd clearcut && git pull --ff-only && docker compose --profile https up -d --build server'
curl -fsS https://145-241-100-85.sslip.io/health
curl -fsS -o /dev/null -w '%{http_code}\n' https://145-241-100-85.sslip.io/
curl -fsS https://145-241-100-85.sslip.io/app.js | grep -c -e "function demoWorkspace" -e "function demoRegisterPage" -e "function demoBulk"
```

Expected: health answers, `/` returns 200, and the served `app.js` contains all three functions (a count of 3). No live tenant, key or document is touched.

- [ ] **Step 8: Record the live result**

Append the exact health response and the grep count to the worklog entry's **Shipped and deployed** line, commit as `docs(worklog): the demo quarter is live`, push, open the docs PR, watch the checks, squash-merge and delete the branch. Switch to `main` and pull.

---

## Self-review

**Spec coverage:**

- §3 fictional SME and quarter: Task 1 data and home.
- §4 the 18 documents, four held types, what each shows, arithmetic anchors: Task 1 data; Task 2 anchors test.
- §5 routes and screens: Tasks 1, 3, 5, 7.
- §5.1 home and library: Task 1; buttons arrive with their screens.
- §5.2 workspace: Task 3.
- §5.3 queue: Task 3.
- §5.4 journey: Task 1.
- §5.5 batch: Task 7.
- §5.6 state and reset: Task 1 (`demoConfirmed`, `demoReset`, strip), exercised in Tasks 3 and 8.
- §6 register grouping, screen, downloads, audit: Task 5.
- §7 isolation and honesty: Global Constraints; Task 2 isolation test; Task 8 network walk.
- §8 visual and accessibility: CSS in Tasks 1, 3, 5, 7; focus, captions, `aria-pressed` and reduced motion in Tasks 1, 3 and 8.
- §9 testing: Tasks 2, 4, 6, 8.
- §10 docs and delivery: Tasks 2, 4, 6, 8.

**Placeholder scan:** no TBD, TODO or "similar to Task N". Every code step carries the code.

**Type consistency:** `demoStatedGross`, `demoResolve`, `demoMoney` and `demoCsvCell` are defined in Task 1 and used unchanged later. `DEMO_KIND`, `DEMO_TAX`, `DEMO_RATE`, `DEMO_KIND_ORDER`, `DEMO_TAX_ORDER`, `DEMO_AUTOMATION_BPS` are defined in Task 1. `rateText` is the existing function at `app.js:1532`. `STATUS` is the existing map at `app.js:60`.
