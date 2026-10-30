# Public demo mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A public `#/demo` journey on two synthetic invoices that needs no key, calls no model, stores nothing, and cannot touch a real account.

**Architecture:** Prepared sample data and demo renderers live in `app.js` beside the real screens, sharing the CSS tokens but no data path. Routing sends `#/demo*` before the key check. The real API, database, worker and CSP are untouched.

**Tech Stack:** Vanilla HTML/CSS/JS served by axum through `include_str!`; Rust string tests in `web.rs`; a Python static server for visual checks.

**Spec:** `docs/superpowers/specs/2026-10-30-public-demo-design.md`

## Global Constraints

- Branch `feat/demo-mode`; conventional commits; one PR; CI green; squash merge. Docs in the same PR.
- No em dashes in UI copy. Plain language. Rand, never USD.
- CSP invariants that `web.rs` tests enforce: exactly one `<script>`, no `http://` or `https://` in `index.html`, no inline handlers or styles, and `app.js` must not contain `innerHTML`, `outerHTML`, `insertAdjacentHTML`, `document.write`, `eval(`, `new Function`, `localStorage`, `sessionStorage`, `document.cookie` or `indexedDB`.
- All document text goes into the page through `h()` (text nodes), never markup.
- Money is integer cents end to end; `demoMoney` only formats an integer for display.
- The demo must never call `api()`, `json()` or `fetch`, never read `key`, and never name a tenant.
- No new server route, no new dependency, no change to `lib.rs`, `web.rs` behaviour, the API, the database or the worker.
- Figures come from `crates/clearcut-eval/sample/g-0003/label.json`, `g-0005/label.json` and the stored reading in `crates/clearcut-eval/sample-results/g-0005.json`. Do not invent amounts.

---

### Task 1: Entry, exit, and the Tafelberg journey

**Files:**
- Modify: `crates/clearcut-server/src/web/index.html:20-26`
- Modify: `crates/clearcut-server/src/web/app.js:10-25` (refs), `:232-234` (hero CTA), `:267-270` (exit handler), insert a demo section after line 270, `:916-925` (routing)
- Modify: `crates/clearcut-server/src/web/app.css` (append demo and glow rules, add one responsive rule to the 900 block)
- Test: `crates/clearcut-server/src/web.rs` (new test after the favicon test)

**Interfaces:**
- Consumes: `h`, `show`, `stopPolling`, `badge`, `timer`, `main`, `location.hash` from `app.js`.
- Produces: `DEMO_NOTE: string`, `DEMO_SAMPLES: object[]` (fields `id`, `file`, `supplier`, `number`, `dateText`, `pages`, `status`, `note`, `lines`, `net`, `vat`, `gross`, `checks`, `report`), `demoSample(id)`, `demoMoney(minor)`, `demoStrip(sample, onRestart)`, `demoCard(sample)`, `demoHome()`, `demoPage(id)`, `demoTimeline(sample, done)`, `demoDocument(sample)`, `demoChecklist(rows)`, `demoReport(sample)`, `demoExportPanel(sample)`, `demoExport(sample, format)`, `demoEndCard()`, and `exitDemoButton`.

- [ ] **Step 1: Write the failing Rust test**

In `crates/clearcut-server/src/web.rs`, after `the_page_and_its_files_are_served_without_a_key_and_locked_down`, add:

```rust
    #[tokio::test]
    async fn the_page_offers_a_demo_that_needs_no_key() {
        let (_, _, html) = fetch("/").await;
        assert!(
            html.contains("href=\"#/demo\""),
            "the header links the demo"
        );
        let (status, _, js) = fetch("/app.js").await;
        assert_eq!(status, StatusCode::OK);
        for marker in [
            "const DEMO_SAMPLES",
            "Sample invoices. Nothing here is saved.",
            "#/demo/",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
    }
```

- [ ] **Step 2: Verify it fails**

Run: `grep -c 'href="#/demo"' crates/clearcut-server/src/web/index.html`

Expected: `0` (no match). Run: `grep -c 'const DEMO_SAMPLES' crates/clearcut-server/src/web/app.js`

Expected: `0`. The new test cannot pass yet.

- [ ] **Step 3: Add the header entry and exit**

In `index.html`, before the about link:

```html
  <a class="nav" href="#/demo">Demo</a>
```

and after the sign-out button:

```html
  <button id="exit-demo" class="link" type="button" hidden>Exit demo</button>
```

- [ ] **Step 4: Wire the button and the hero CTA in `app.js`**

After `const footerLinks = ...`, add:

```js
const exitDemoButton = document.getElementById("exit-demo");
```

After the `signoutButton` listener, add:

```js
exitDemoButton.addEventListener("click", () => {
  location.hash = "#/";
});
```

Replace the hero CTA row (currently "See how it works" and "Read the source") with:

```js
        h("div", { class: "ctas" },
          h("a", { class: "button cta-glow", href: "#/demo" }, "Try the live demo"),
          h("a", { class: "button secondary", href: "#how" }, "See how it works"),
          h("a", { href: REPO, rel: "noreferrer" }, "Read the source")),
```

- [ ] **Step 5: Add the demo data and renderers**

Insert after the sign-out handler and before `// ---- 2. documents ----`:

```js
// ---------------------------------------------------------------- 1b. the demo ----
// Prepared from two synthetic samples in the repository (crates/clearcut-eval/sample:
// g-0003 and g-0005). Nothing here calls the API, holds a key or names a tenant: the demo
// is data in this file and state in this page, so it cannot see or touch an account.

const DEMO_NOTE = "Sample invoices. Nothing here is saved.";

const DEMO_SAMPLES = [
  {
    id: "tp-4410",
    file: "tafelberg-tp-4410.pdf",
    supplier: "Tafelberg Print Works CC",
    supplierVat: "4999900102",
    number: "TP-4410",
    dateText: "3 March 2026",
    pages: 3,
    status: "done",
    note: "Nine standard-rated lines over three pages, read and checked to the cent.",
    lines: [
      { page: 1, description: "Business cards, box of 500", quantity: 10, unit: 18500, total: 185000, tax: "standard" },
      { page: 1, description: "Flyers A5, 1000", quantity: 4, unit: 42000, total: 168000, tax: "standard" },
      { page: 1, description: "Brochures tri-fold", quantity: 5, unit: 65000, total: 325000, tax: "standard" },
      { page: 1, description: "Posters A2", quantity: 12, unit: 9500, total: 114000, tax: "standard" },
      { page: 2, description: "Banners 2m", quantity: 3, unit: 78000, total: 234000, tax: "standard" },
      { page: 2, description: "Stickers, roll", quantity: 8, unit: 14000, total: 112000, tax: "standard" },
      { page: 2, description: "Letterheads, ream", quantity: 6, unit: 31000, total: 186000, tax: "standard" },
      { page: 2, description: "Envelopes DL, box", quantity: 9, unit: 12000, total: 108000, tax: "standard" },
      { page: 3, description: "Design setup fee", quantity: 1, unit: 150000, total: 150000, tax: "standard" },
    ],
    net: 1582000,
    vat: 237300,
    gross: 1819300,
    checks: [
      ["Every line", "Passed", "Each line is quantity times unit price to the cent, over pages 1 to 3."],
      ["VAT at 15%", "Passed", "R 2 373.00 on R 15 820.00 of standard-rated lines."],
      ["Net plus VAT", "Passed", "R 15 820.00 plus R 2 373.00 is R 18 193.00, as printed."],
      ["SA tax-invoice fields", "Passed", "Supplier and recipient VAT numbers, invoice number and date are present."],
    ],
    report: [
      ["What this is", "Tafelberg Print Works CC invoiced Sample Client (Pty) Ltd R 18 193.00 on 3 March 2026, invoice TP-4410."],
      ["What was read", "Nine standard-rated lines over three pages. The largest line is brochures at R 3 250.00."],
      ["What was checked", "Every line total, VAT at 15%, net plus VAT and the SA tax-invoice fields agree exactly with the page."],
    ],
  },
];

function demoSample(id) {
  return DEMO_SAMPLES.find((sample) => sample.id === id) || null;
}

function demoMoney(minor) {
  const sign = minor < 0 ? "-" : "";
  const abs = Math.abs(minor);
  const whole = String(Math.floor(abs / 100)).replace(/\B(?=(\d{3})+(?!\d))/g, " ");
  return sign + "R " + whole + "." + String(abs % 100).padStart(2, "0");
}

function demoStrip(sample, onRestart) {
  return h("div", { class: "demo-strip" },
    h("span", { class: "demo-chip" }, "Demo"),
    h("span", { class: "demo-strip-text" }, DEMO_NOTE),
    h("span", { class: "demo-actions" },
      h("button", { type: "button", class: "link", onclick: () => { location.hash = "#/"; } }, "Exit demo"),
      sample && onRestart ? h("button", { type: "button", class: "link", onclick: onRestart }, "Start over") : null));
}

function demoCard(sample) {
  return h("a", { class: "sample-card", href: "#/demo/" + sample.id },
    h("span", { class: "sample-file" }, sample.file),
    h("strong", {}, sample.supplier),
    h("span", { class: "small muted" }, "Invoice " + sample.number + " · " + sample.pages + (sample.pages === 1 ? " page" : " pages") + " · " + demoMoney(sample.gross)),
    badge(sample.status),
    h("span", { class: "small" }, sample.note));
}

function demoHome() {
  stopPolling();
  show(h("div", { class: "page demo" },
    demoStrip(null, null),
    h("div", { class: "page-head" },
      h("p", { class: "eyebrow" }, "Demo"),
      h("h1", {}, "An invoice, read end to end."),
      h("p", { class: "lede" }, "This is a made-up invoice from ClearCut's test set. The reading, the timeline and the checks are prepared in advance: no model is called, nothing is uploaded and nothing here is saved.")),
    h("div", { class: "sample-grid" }, DEMO_SAMPLES.map(demoCard)),
    h("p", { class: "small muted" }, "In the real app a model reads each page, every figure is checked against the page it came from, and what cannot be settled goes to a person. ClearCut is invite-only while in alpha: ask for a key from whoever sent you this link.")));
}

function demoPage(id) {
  stopPolling();
  const sample = demoSample(id);
  if (!sample) {
    location.hash = "#/demo";
    return;
  }
  const state = { phase: "timeline", confirmed: false };
  const stage = h("div", {});
  const draw = () => {
    if (state.phase === "timeline") {
      stage.replaceChildren(demoTimeline(sample, () => { state.phase = "document"; draw(); }));
    } else {
      stage.replaceChildren(demoDocument(sample));
    }
  };
  show(h("div", { class: "page demo" },
    demoStrip(sample, () => { location.hash = "#/demo"; }),
    h("p", { class: "small" }, h("a", { href: "#/demo" }, "← All samples")),
    stage));
  draw();
}

function demoTimeline(sample, done) {
  const steps = ["File accepted", sample.pages + (sample.pages === 1 ? " page found" : " pages found")];
  for (let page = 1; page <= sample.pages; page += 1) steps.push("Reading page " + page + " of " + sample.pages);
  steps.push("Checking every figure", "Done");
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
    h("h1", {}, sample.file),
    h("p", { class: "small muted" }, "This timeline is simulated. No model is called and no file is uploaded."),
    progress,
    status,
    list,
    h("div", { class: "actions" },
      h("button", { type: "button", class: "secondary", onclick: () => { stopPolling(); done(); } }, "Skip to the result")));
}

function demoChecklist(rows) {
  return h("div", { class: "checks" },
    rows.map(([label, verdict, detail]) =>
      h("div", { class: "check " + (verdict === "Passed" ? "pass" : "fail") },
        h("span", { class: "check-verdict" }, verdict),
        h("strong", {}, label),
        h("span", { class: "small muted" }, detail))));
}

function demoReport(sample) {
  return h("div", { class: "report" },
    sample.report.map(([heading, text]) => h("div", {}, h("h3", {}, heading), h("p", {}, text))));
}

function demoDocument(sample) {
  return h("div", {},
    h("div", { class: "row" }, h("h1", {}, sample.file), badge(sample.status)),
    h("p", { class: "small muted" },
      sample.supplier + " · Invoice " + sample.number + " · " + sample.dateText + " · " +
      sample.pages + (sample.pages === 1 ? " page" : " pages") + " · prepared sample"),
    h("h2", {}, "Figures checked"),
    demoChecklist(sample.checks),
    h("h2", {}, "Report"),
    demoReport(sample),
    h("h2", {}, "Export"),
    demoExportPanel(sample),
    demoEndCard());
}

function demoExport(sample, format) {
  const cell = (value) => '"' + String(value).replaceAll('"', '""') + '"';
  if (format === "csv") {
    const rows = [[
      "# ClearCut demo. Sample data, not a real invoice.",
      ["supplier", "invoice", "date", "page", "description", "quantity", "unit", "line_total", "tax"].map(cell).join(","),
    ].join("\n")];
    for (const line of sample.lines) {
      rows.push([sample.supplier, sample.number, sample.dateText, line.page, line.description, line.quantity,
        demoMoney(line.unit), demoMoney(line.total), line.tax].map(cell).join(","));
    }
    rows.push(["", "", "", "", "Net", "", "", demoMoney(sample.net), ""].map(cell).join(","));
    rows.push(["", "", "", "", "VAT", "", "", demoMoney(sample.vat), ""].map(cell).join(","));
    rows.push(["", "", "", "", "Total", "", "", demoMoney(sample.gross), ""].map(cell).join(","));
    return rows.join("\n");
  }
  const lines = sample.lines.map((line) =>
    "| " + line.page + " | " + line.description + " | " + line.quantity + " | " + demoMoney(line.unit) + " | " + demoMoney(line.total) + " |");
  return [
    "# ClearCut demo: sample data, not a real invoice",
    "",
    "**" + sample.supplier + "** · invoice " + sample.number + " · " + sample.dateText,
    "",
    "| Page | Description | Qty | Unit | Line |",
    "|---:|---|---:|---:|---:|",
    ...lines,
    "",
    "Net " + demoMoney(sample.net),
    "VAT " + demoMoney(sample.vat),
    "**Total " + demoMoney(sample.gross) + "**",
  ].join("\n");
}

function demoExportPanel(sample) {
  const out = h("pre", { class: "demo-output", tabindex: "0" }, "Choose CSV or Markdown.");
  const note = h("p", { class: "small muted", role: "status" }, "");
  const build = (format) => {
    out.textContent = demoExport(sample, format);
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

- [ ] **Step 6: Route the demo before the key check**

Replace `route()` with:

```js
function route() {
  const hash = location.hash || "#/";
  stopPolling();
  exitDemoButton.hidden = !hash.startsWith("#/demo");
  if (hash === "#/demo") return demoHome();
  const sample = hash.match(/^#\/demo\/([a-z0-9-]+)$/);
  if (sample) return demoPage(sample[1]);
  if (!key) return home("");
  setSignedIn(true);
  const match = hash.match(/^#\/d\/([0-9a-f-]{36})$/i);
  if (match) documentPage(match[1]);
  else if (hash === "#/settings") settingsPage();
  else if (hash === "#/about") aboutPage();
  else documents();
}
```

Note: the demo branch returns before `if (!key)`, so the demo opens signed out, and it never reads `key` and never calls `setSignedIn`.

- [ ] **Step 7: Add the glow and the demo layout CSS**

Append before the existing `/* ---- small ---- */` block:

```css
/* ---------------------------------------------------------------- demo ----
   The demo is prepared data and local state: it shares these tokens and nothing else. */
.cta-glow {
  position: relative;
  animation: glow 4.5s ease-in-out infinite;
}
@keyframes glow {
  0%, 100% {
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent) 22%, transparent),
                0 6px 22px -8px color-mix(in srgb, var(--accent) 55%, transparent);
  }
  50% {
    box-shadow: 0 0 0 5px color-mix(in srgb, var(--accent) 12%, transparent),
                0 12px 34px -6px color-mix(in srgb, var(--accent) 66%, transparent);
  }
}
@media (prefers-reduced-motion: reduce) {
  .cta-glow { animation: none; box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 14%, transparent); }
}
.demo-strip {
  display: flex; align-items: center; gap: 12px; flex-wrap: wrap;
  padding: 12px 16px; margin-bottom: 22px;
  background: var(--accent-soft); border: 1px solid var(--rule-strong); border-radius: var(--radius);
}
.demo-chip {
  font: 700 11px/1 var(--sans); letter-spacing: 0.14em; text-transform: uppercase;
  color: var(--accent-ink); background: var(--accent); border-radius: 999px; padding: 6px 10px;
}
.demo-strip-text { color: var(--ink-soft); font-size: 14px; }
.demo-actions { margin-left: auto; display: flex; gap: 14px; flex-wrap: wrap; }
.sample-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 18px; margin: 26px 0 30px; }
.sample-card {
  display: grid; gap: 6px; justify-items: start;
  padding: 22px; background: var(--surface); border: 1px solid var(--rule); border-radius: var(--radius);
  color: var(--ink); text-decoration: none; box-shadow: var(--shadow);
  transition: border-color 120ms ease, transform 120ms ease;
}
.sample-card:hover { border-color: var(--accent); transform: translateY(-2px); }
.sample-card strong { font-family: var(--serif); font-size: 21px; }
.sample-file { font: 12.5px/1 var(--mono); color: var(--muted); overflow-wrap: anywhere; }
.timeline { list-style: none; padding: 0; margin: 18px 0; display: grid; gap: 10px; }
.timeline li { padding-left: 26px; position: relative; color: var(--ink-soft); }
.timeline li::before {
  content: ""; position: absolute; left: 4px; top: 0.55em;
  width: 8px; height: 8px; border-radius: 50%; background: var(--accent);
}
.demo-progress { max-width: 520px; }
.checks { display: grid; gap: 1px; background: var(--rule); border: 1px solid var(--rule); border-radius: var(--radius); overflow: hidden; }
.check { display: grid; grid-template-columns: 92px minmax(0, 1fr); gap: 4px 14px; padding: 13px 16px; background: var(--surface); }
.check-verdict { font: 700 12px/1.5 var(--sans); letter-spacing: 0.06em; text-transform: uppercase; }
.check.pass .check-verdict { color: var(--done); }
.check.fail .check-verdict { color: var(--warn); }
.check strong { font-size: 15px; }
.check .small { grid-column: 2; }
.demo-output {
  margin: 14px 0 0; padding: 16px; max-height: 320px; overflow: auto;
  background: var(--paper); border: 1px solid var(--rule); border-radius: 10px;
  font: 12.5px/1.6 var(--mono); color: var(--ink-soft); white-space: pre;
}
.demo-end { margin-top: 30px; }
```

In the existing `@media (max-width: 900px)` block add:

```css
  .sample-grid { grid-template-columns: 1fr; }
```

- [ ] **Step 8: Check the script and the markers**

Run: `node --check crates/clearcut-server/src/web/app.js`

Expected: no output (exit 0). Run: `grep -c 'href="#/demo"' crates/clearcut-server/src/web/index.html` and `grep -c 'const DEMO_SAMPLES' crates/clearcut-server/src/web/app.js`

Expected: `1` and `1`.

- [ ] **Step 9: Watch the journey in a browser**

Run: `python -m http.server 8098 --directory crates/clearcut-server/src/web`

Open `http://127.0.0.1:8098/index.html#/demo`, choose the Tafelberg card, and confirm: the timeline steps advance, Skip jumps to the document, the four checks pass, the report reads, Show CSV and Show Markdown fill the panel, Copy works, Exit demo returns to the landing page, and the header Exit is hidden on the landing page. Open the browser console: no errors.

- [ ] **Step 10: Commit**

```bash
git add crates/clearcut-server/src/web/index.html crates/clearcut-server/src/web/app.js crates/clearcut-server/src/web/app.css crates/clearcut-server/src/web.rs
git commit -m "feat(web): a public demo door, and the first sample read end to end"
```

---

### Task 2: The Bergsig review journey

**Files:**
- Modify: `crates/clearcut-server/src/web/app.js` (`DEMO_SAMPLES`, `demoHome` copy, `demoPage` draw, `demoDocument`, `demoChecklist`, `demoReport`)
- Modify: `crates/clearcut-server/src/web/app.css` (review split, invoice sheet, responsive rule)

**Interfaces:**
- Consumes: everything Task 1 produced.
- Produces: a second `DEMO_SAMPLES` entry with `grossRead`, `grossPrinted`, `confirmedChecks`, `reportConfirmed`; `demoReview(sample, state, redraw)`; `demoDocument(sample, state)`; `demoChecklist(rows)` unchanged; `demoReport(sample, confirmed)`.

- [ ] **Step 1: Add the second sample**

Append to `DEMO_SAMPLES` after the Tafelberg object:

```js
  {
    id: "bst-221",
    file: "bergsig-bst-221.pdf",
    supplier: "Bergsig Schools Trust",
    supplierVat: "4999900104",
    number: "BST-221",
    dateText: "2 April 2026",
    dueText: "16 April 2026",
    pages: 1,
    status: "needs_review",
    note: "One exempt line, and a total the reader got wrong by one rand.",
    lines: [
      { page: 1, description: "Tuition fees, term 2", quantity: 1, unit: 1250000, total: 1250000, tax: "exempt" },
    ],
    net: 1250000,
    vat: 0,
    gross: 1250000,
    grossRead: 1250100,
    grossPrinted: 1250000,
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
      ["What this is", "Bergsig Schools Trust invoiced Sample Client (Pty) Ltd for tuition, invoice BST-221, dated 2 April 2026."],
      ["What was read", "One exempt line: tuition for the second term, R 12 500.00."],
      ["Why it waits", "The reading gives the total as R 12 501.00. Net plus VAT is R 12 500.00, and the page prints R 12 500.00, so the figures do not agree."],
    ],
    reportConfirmed: [
      ["What this is", "Bergsig Schools Trust invoiced Sample Client (Pty) Ltd for tuition, invoice BST-221, dated 2 April 2026."],
      ["What was read", "One exempt line: tuition for the second term, R 12 500.00."],
      ["What was checked", "With the printed total of R 12 500.00, net plus VAT agrees exactly and the page's figure is the one recorded."],
    ],
  },
```

- [ ] **Step 2: Make the home copy plural**

In `demoHome`, change the h1 to `"Two invoices, read end to end."` and the lede to `"These are made-up invoices from ClearCut's test set. The readings, the timelines and the checks are prepared in advance: no model is called, nothing is uploaded and nothing here is saved."`

- [ ] **Step 3: Teach the document view about held pages**

Replace `demoDocument` with:

```js
function demoDocument(sample, state) {
  const held = sample.status === "needs_review" && !state.confirmed;
  const parts = [
    h("div", { class: "row" }, h("h1", {}, sample.file), badge(held ? "needs_review" : "done")),
    h("p", { class: "small muted" },
      sample.supplier + " · Invoice " + sample.number + " · " + sample.dateText + " · " +
      sample.pages + (sample.pages === 1 ? " page" : " pages") + " · prepared sample"),
  ];
  if (held) {
    parts.push(
      h("div", { class: "banner", role: "note" },
        h("strong", {}, "A person needs to look at this document. "),
        "The reading says the total is " + demoMoney(sample.grossRead) + "; the page prints " + demoMoney(sample.grossPrinted) + ".",
        h("br"),
        h("span", { class: "small muted" }, "Nothing was sent to a model and nothing is stored: this is the demo's prepared reading."),
        h("div", { class: "actions" },
          h("button", { type: "button", onclick: () => { state.phase = "review"; state.redraw(); } }, "Review this page"))));
  }
  if (sample.status === "needs_review" && state.confirmed) {
    parts.push(
      h("div", { class: "banner", role: "status" },
        h("strong", {}, "Confirmed by you in this demo. "),
        "In the real app the correction would be recorded as yours, against your key, without a model call."));
  }
  parts.push(h("h2", {}, "Figures checked"), demoChecklist(demoChecks(sample, state.confirmed)));
  parts.push(h("h2", {}, "Report"), demoReport(sample, state.confirmed));
  if (!held) {
    parts.push(h("h2", {}, "Export"), demoExportPanel(sample, state.confirmed));
  }
  parts.push(demoEndCard());
  return h("div", {}, parts);
}

function demoChecks(sample, confirmed) {
  return confirmed && sample.confirmedChecks ? sample.confirmedChecks : sample.checks;
}
```

`state.redraw` is set in `demoPage` (Step 5), so the button does not need a captured `draw`.

- [ ] **Step 4: Use the confirmed figures in the export and report**

Replace `demoReport` with:

```js
function demoReport(sample, confirmed) {
  const rows = confirmed && sample.reportConfirmed ? sample.reportConfirmed : sample.report;
  return h("div", { class: "report" },
    rows.map(([heading, text]) => h("div", {}, h("h3", {}, heading), h("p", {}, text))));
}
```

In `demoExportPanel`, change the signature to `demoExportPanel(sample, confirmed)` and both `demoExport(sample, format)` calls to `demoExport(sample, confirmed, format)`. In `demoExport`, change the signature to `demoExport(sample, confirmed, format)` and use `const total = confirmed ? sample.grossPrinted ?? sample.gross : sample.gross;` then replace both `demoMoney(sample.gross)` totals (CSV total row and Markdown bold total) with `demoMoney(total)`.

For Tafelberg, `confirmed` is false and `grossPrinted` is undefined, so the total stays R 18 193.00.

- [ ] **Step 5: Wire the review phase and the redraw**

Replace `demoPage` with:

```js
function demoPage(id) {
  stopPolling();
  const sample = demoSample(id);
  if (!sample) {
    location.hash = "#/demo";
    return;
  }
  const state = { phase: "timeline", confirmed: false, redraw: null };
  const stage = h("div", {});
  const draw = () => {
    if (state.phase === "timeline") {
      stage.replaceChildren(demoTimeline(sample, () => { state.phase = "document"; draw(); }));
    } else if (state.phase === "review") {
      stage.replaceChildren(demoReview(sample, state, draw));
    } else {
      stage.replaceChildren(demoDocument(sample, state));
    }
  };
  state.redraw = draw;
  show(h("div", { class: "page demo" },
    demoStrip(sample, () => { location.hash = "#/demo"; }),
    h("p", { class: "small" }, h("a", { href: "#/demo" }, "← All samples")),
    stage));
  draw();
}
```

- [ ] **Step 6: Add the review screen**

Add after `demoDocument`:

```js
function demoReview(sample, state, redraw) {
  const back = () => { state.phase = "document"; redraw(); };
  const sheet = h("div", { class: "invoice-sheet", "aria-label": "Sample invoice " + sample.number },
    h("div", { class: "invoice-head" },
      h("div", {},
        h("strong", {}, sample.supplier),
        h("span", { class: "small muted" }, "3 Kerk Street, Stellenbosch, 7600"),
        h("span", { class: "small muted" }, "VAT " + sample.supplierVat)),
      h("div", { class: "invoice-meta" },
        h("span", {}, "Invoice " + sample.number),
        h("span", {}, sample.dateText),
        h("span", {}, "Due " + sample.dueText))),
    h("table", {},
      h("thead", {}, h("tr", {}, h("th", {}, "Description"), h("th", { class: "num" }, "Qty"), h("th", { class: "num" }, "Amount"))),
      h("tbody", {},
        sample.lines.map((line) =>
          h("tr", {},
            h("td", {}, line.description),
            h("td", { class: "num" }, line.quantity),
            h("td", { class: "num" }, demoMoney(line.total)))))),
    h("div", { class: "invoice-totals" },
      h("div", {}, h("span", {}, "Net"), h("b", {}, demoMoney(sample.net))),
      h("div", {}, h("span", {}, "VAT (exempt)"), h("b", {}, demoMoney(sample.vat))),
      h("div", { class: "due" }, h("span", {}, "Total"), h("b", {}, h("mark", {}, demoMoney(sample.grossPrinted))))));
  return h("div", {},
    h("div", { class: "row" }, h("h1", {}, "Review " + sample.file), badge("needs_review")),
    h("p", { class: "small muted" }, "This page is a recreation of the made-up sample invoice, so it reads the same in dark mode."),
    h("div", { class: "review-split" },
      sheet,
      h("div", { class: "card" },
        h("h2", {}, "What the reader says"),
        h("dl", { class: "reading" },
          h("dt", {}, "Invoice number"), h("dd", {}, sample.number),
          h("dt", {}, "Date"), h("dd", {}, sample.dateText),
          h("dt", {}, "Total (gross)"), h("dd", { class: "error" }, demoMoney(sample.grossRead))),
        h("div", { class: "banner", role: "note" },
          h("strong", {}, "The total does not match. "),
          "The reading says " + demoMoney(sample.grossRead) + ". The page prints " + demoMoney(sample.grossPrinted) +
          ", and net plus VAT is " + demoMoney(sample.net + sample.vat) + "."),
        h("p", { class: "small muted" }, "In the real app this is a form: the correction is checked by the same rules as a model's reading, recorded as yours and costs nothing."),
        h("div", { class: "row" },
          h("button", { type: "button", onclick: () => { state.confirmed = true; back(); } }, "Confirm " + demoMoney(sample.grossPrinted)),
          h("button", { type: "button", class: "secondary", onclick: back }, "Back to the document")))));
}
```

- [ ] **Step 7: Style the review screen**

Append before `/* ---- small ---- */`:

```css
.review-split { display: grid; grid-template-columns: minmax(0, 1.1fr) minmax(0, 0.9fr); gap: 22px; align-items: start; }
.invoice-sheet {
  background: var(--surface); border: 1px solid var(--rule-strong); border-radius: var(--radius);
  padding: 26px; box-shadow: var(--shadow); font-size: 14px;
}
.invoice-head { display: flex; justify-content: space-between; gap: 18px; flex-wrap: wrap; margin-bottom: 18px; }
.invoice-head strong { font-family: var(--serif); font-size: 20px; display: block; }
.invoice-head span { display: block; }
.invoice-meta { text-align: right; }
.invoice-sheet th, .invoice-sheet td { padding: 9px 10px; }
.invoice-totals { margin-top: 14px; margin-left: auto; width: min(260px, 100%); display: grid; gap: 6px; }
.invoice-totals div { display: flex; justify-content: space-between; gap: 12px; }
.invoice-totals .due { border-top: 1px solid var(--rule-strong); padding-top: 8px; font-size: 17px; }
.invoice-sheet mark { background: var(--warn-bg); color: var(--warn); padding: 0 4px; border-radius: 4px; }
.reading { display: grid; grid-template-columns: 1fr auto; gap: 8px 16px; margin: 0 0 16px; }
.reading dt { color: var(--muted); }
.reading dd { margin: 0; text-align: right; font-variant-numeric: tabular-nums; }
```

In the `@media (max-width: 900px)` block add:

```css
  .review-split { grid-template-columns: 1fr; }
```

- [ ] **Step 8: Check the script and walk the review**

Run: `node --check crates/clearcut-server/src/web/app.js`

Expected: no output. Open `http://127.0.0.1:8098/index.html#/demo` (restart the Python server if it stopped), open Bergsig: the timeline ends "Held for a person", the banner names both figures, Review this page opens the sheet with the printed total marked, Confirm shows Done, the checks switch to Passed, the report changes, and the export total is R 12 500.00. Back to the document from the review writes nothing and keeps the held state.

- [ ] **Step 9: Commit**

```bash
git add crates/clearcut-server/src/web/app.js crates/clearcut-server/src/web/app.css
git commit -m "feat(web): a demo document that needs a person, and a review that settles it"
```

---

### Task 3: Prove the boundaries, document entry, and go live

**Files:**
- Modify: `docs/ADMIN.md`, `docs/LAUNCH.md`, `docs/WORKLOG.md`
- No code changes unless a check fails.

**Interfaces:**
- Consumes: the finished demo.
- Produces: the documented invite-only entry path and the live demo.

- [ ] **Step 1: Prove the demo never touches the API**

In Chrome, open the live or local page with DevTools, Network, Fetch/XHR filter. Walk both samples end to end: timeline, Skip, checks, report, both exports, review, Confirm, Exit. Expected: zero requests to `/v1/*`, and no request carries an `Authorization` header. The only requests are the page, `app.css`, `app.js` and `favicon.svg`.

Then sign in with an admin key on the local stack (or the live one), open `#/demo`, walk it, and confirm the network panel still shows zero `/v1` requests. Exit returns to the documents list.

- [ ] **Step 2: Check the screens and keyboard**

At 1280 and 390 wide, light and dark: the demo strip stays readable, the sample cards stack, the invoice sheet and total sit side by side then stacked, nothing overflows. With reduced motion emulated, the glow is static and the timeline still advances. Keyboard only: Tab reaches Demo, the sample card, Skip, Show CSV, Show Markdown, Copy, Review, Confirm and Exit; each has a visible focus ring.

- [ ] **Step 3: Document inviting someone**

In `docs/ADMIN.md`, after "First-time setup of a tenant", add:

```markdown
## Inviting someone
ClearCut is invite-only in alpha: the public demo shows the journey, and a real account is made here, one at a time.

```
export DATABASE_URL=<the OWNER connection string>
TENANT=$(clearcut-admin create-tenant "Their business")
clearcut-admin create-key --tenant "$TENANT" --name "Anna's first key" \
    --scopes keys:admin,documents:write,events:read > anna.key
```

- Send the key through a channel you trust, such as a password-manager share. Not a group chat, and not the same email as the invoices.
- One key per person, named for them, so a departure is one `revoke-key --tenant ... --key ...` and nothing else is disturbed.
- They open the site, paste the key, then go to Settings and add their own Anthropic key with a spend limit.
- The public demo at `#/demo` needs no key, calls no model and stores nothing. It is the right link to send before someone has a key.
```

- [ ] **Step 4: Update the launch checklist**

In `docs/LAUNCH.md` section 1, after the web-page bullet, add:

```markdown
- **A public demo** at `#/demo`: two synthetic invoices read end to end, one clean and one held for a person, with a played-back timeline, the checks, the report, a browser-built export and a review that confirms. It needs no key, calls no model, stores nothing and cannot reach a tenant; accounts remain invite-only.
```

In section 3 add:

```markdown
- **Invitations**: ClearCut is invite-only in alpha. You create a tenant and a key for each person; the exact commands are in [ADMIN.md](ADMIN.md).
```

In section 5, add:

```markdown
- **The demo is prepared data, not a live reading.** It shows the screens and the story of the checks; it is not evidence of the reader's accuracy, which still waits on the gold set (P2-01).
```

- [ ] **Step 5: Add the worklog entry**

At the top of `docs/WORKLOG.md`, above part 72:

```markdown
## 2026-10-30 (part 73) — a public demo that cannot touch an account
**Done**
- **`#/demo`, open without a key.** Two synthetic samples from `crates/clearcut-eval/sample`: Tafelberg TP-4410 (three pages, nine standard-rated lines, ends done) and Bergsig BST-221 (one exempt line, held because the reading says R 12 501.00 where the page prints R 12 500.00). Each plays a short, skippable timeline, then shows the checks, the report and an export built in the browser.
- **The review is the point of the second sample.** The sheet recreates the page, the reading sits beside it, and Confirm settles it locally. The screen says the real app records a confirmation as the person's, against their key, with no model call.
- **Why it cannot mix with live data.** The demo is data and state in `app.js`: no `api()` call, no key, no tenant, no database, no model. `#/demo*` is routed before the key check. All document text still goes through `h()`; the CSP and the one-script rule are unchanged. Checked with the network panel open: zero `/v1` requests from either sample, signed out and signed in.
- **Entry stays invite-only.** The hero has a glowing "Try the live demo" button and the header a Demo link; the demo strip and the header button exit to `#/`. ADMIN.md now has the invite steps, LAUNCH records both, and the demo's limits are written down.
**Not done:** the demo is not a live reading, so it says nothing about accuracy; that waits on the gold set.
```

- [ ] **Step 6: Run the local gates**

Run: `node --check crates/clearcut-server/src/web/app.js`

Run: `cargo test -p clearcut-server --lib web::`

Expected: all web tests pass, including `the_page_offers_a_demo_that_needs_no_key`. If this Windows machine cannot build the provider macro, record it and rely on the rust CI job for this test; the grep checks in Task 1 are the local proof.

Run: `grep -n 'innerHTML\|outerHTML\|insertAdjacentHTML\|document.write\|eval(\|new Function\|localStorage\|sessionStorage\|document.cookie\|indexedDB' crates/clearcut-server/src/web/app.js`

Expected: no output.

- [ ] **Step 7: Commit, push, and watch CI**

```bash
git add docs/ADMIN.md docs/LAUNCH.md docs/WORKLOG.md
git commit -m "docs: how someone is invited, and what the demo does not prove"
git push -u origin feat/demo-mode
gh pr create --title "feat(web): a public demo that cannot touch an account" --body "$(cat <<'EOF'
## Summary
- `#/demo` opens without a key: two synthetic invoices, a played-back timeline, the checks, the report, a browser-built export, and a review that settles a R1.00 misread.
- Entry stays invite-only; the hero gets a glowing demo button and the header a Demo link. The demo strip and the header button exit to `#/`.
- The demo is prepared data and local state in app.js: no API call, no key, no tenant, no model. CSP and the one-script rule unchanged.

## Test plan
- [x] `node --check`; no forbidden JavaScript APIs
- [x] Network panel: zero `/v1` requests through both journeys, signed out and signed in
- [x] 1280 and 390, light and dark, reduced motion, keyboard
- [ ] CI (rust web tests, compose)
EOF
)"
gh pr checks --watch --interval 20
```

Expected: every check passes. Then:

```bash
gh pr merge --squash --delete-branch
git checkout main
git pull
```

- [ ] **Step 8: Deploy and verify live**

```bash
ssh -i target/deploy/clearcut_oracle ubuntu@145.241.100.85 \
  'cd /home/ubuntu/clearcut && git pull --ff-only -q && git log --oneline -1 && sudo docker compose --profile https up -d --build server'
curl -sS -o /dev/null -w '%{http_code}\n' https://145-241-100-85.sslip.io/app.js
```

Expected: HTTP 200. In the browser, open https://145-241-100-85.sslip.io/#/demo with the network panel open, walk both samples, confirm zero `/v1` requests, and exit. Then open the landing page and check the glowing button at 1280 and 390.

---

## Self-review

- Spec coverage: entry and exit (Task 1), samples and timeline (1 and 2), review and export (1 and 2), isolation and honesty (Task 3 step 1, plus the strip and copy), visual and accessibility (1, 2, 3), testing (1 and 3), docs (3), rejected alternatives (spec only, not built).
- Every function referenced is defined in a task: `demoReview` in Task 2 step 6 before its use in step 5's draw; `state.redraw` set in step 5; `demoChecks` and `demoReport` changed in Task 2 steps 3 and 4 with every call site updated.
- Money stays integer cents; only `demoMoney` formats.
- The demo has no server, key, tenant or fetch path, so no new isolation code is required.
