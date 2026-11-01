// Written by Kone Tshivhinda & Claude.
// Three screens, no framework. Everything that came off a document goes into the page with
// textContent (see h() below), never as markup. The ClearCut key lives in this variable only:
// it is not written to storage or a cookie, so reloading the page signs you out.
"use strict";

let key = null;
let timer = null;
const REPO = "https://github.com/konethegreat/clearcut";
const main = document.getElementById("main");
const signoutButton = document.getElementById("signout");
const settingsLink = document.getElementById("settings-link");
const registerLink = document.getElementById("register-link");
const aboutLink = document.getElementById("about-link");
const footerLinks = document.getElementById("footer-links");
const exitDemoButton = document.getElementById("exit-demo");
const demoLink = document.getElementById("demo-link");

function setSignedIn(signedIn) {
  signoutButton.hidden = !signedIn;
  settingsLink.hidden = !signedIn;
  registerLink.hidden = !signedIn;
  aboutLink.hidden = !signedIn;
}

footerLinks.append(
  h("a", { href: "#/trust" }, "Trust"),
  document.createTextNode(" · "),
  h("a", { href: REPO, rel: "noreferrer" }, "Source on GitHub"),
  document.createTextNode(" · Apache-2.0"),
);

// ---------------------------------------------------------------- helpers ----

// Builds an element. Strings become text nodes (never markup).
function h(tag, attrs, ...children) {
  const el = document.createElement(tag);
  for (const [name, value] of Object.entries(attrs || {})) {
    if (name === "class") el.className = value;
    else if (name.startsWith("on")) el.addEventListener(name.slice(2), value);
    else if (value === true) el.setAttribute(name, "");
    else if (value !== false && value != null) el.setAttribute(name, value);
  }
  for (const child of children.flat()) {
    if (child == null || child === false) continue;
    el.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return el;
}

function show(...nodes) {
  main.replaceChildren(...nodes);
}

function stopPolling() {
  if (timer) clearTimeout(timer);
  timer = null;
}

const STATUS = { received: "Waiting", processing: "Reading", needs_review: "Needs review", done: "Done" };

function badge(status) {
  return h("span", { class: "badge " + status }, STATUS[status] || status);
}

function when(iso) {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleString("en-ZA", { dateStyle: "medium", timeStyle: "short" });
}

class AuthError extends Error {}

// Calls the API with the key. A 401 sends you back to sign in.
async function api(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    headers: { Authorization: "Bearer " + key, ...(options.headers || {}) },
  });
  if (response.status === 401) throw new AuthError("The key was not accepted.");
  if (response.status === 403) throw new Error("This key is not allowed to do that.");
  return response;
}

async function json(path) {
  const response = await api(path);
  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new Error(body.error || "Something went wrong (" + response.status + ").");
  }
  return response.json();
}

// Saves a response as a file, under the name the server chose when it gave one.
async function saveFile(response, fallback) {
  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new Error(body.error || "The download failed (" + response.status + ").");
  }
  const found = (response.headers.get("Content-Disposition") || "").match(/filename="([^"]+)"/);
  const url = URL.createObjectURL(await response.blob());
  const link = h("a", { href: url, download: found ? found[1] : fallback });
  document.body.append(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10000);
}

function failure(error, retry) {
  if (error instanceof AuthError) return signIn(error.message);
  return h(
    "div",
    { class: "states" },
    h("p", { class: "error" }, error.message || "Something went wrong."),
    retry ? h("button", { type: "button", class: "secondary", onclick: retry }, "Try again") : null,
  );
}

// ---------------------------------------------------------------- 1. home and sign in ----

function ticker(el, to, prefix, suffix) {
  const format = new Intl.NumberFormat("en-ZA");
  const done = () => { el.textContent = prefix + format.format(to) + suffix; };
  done();
  if (matchMedia("(prefers-reduced-motion: reduce)").matches || !("IntersectionObserver" in window)) return;
  const watch = new IntersectionObserver((entries) => {
    if (!entries.some((entry) => entry.isIntersecting)) return;
    watch.disconnect();
    const start = performance.now();
    const step = (now) => {
      const t = Math.min(1, (now - start) / 900);
      const eased = 1 - Math.pow(1 - t, 3);
      el.textContent = prefix + format.format(Math.round(to * eased)) + suffix;
      if (t < 1) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }, { threshold: 0.4 });
  watch.observe(el);
}

function marquee() {
  const items = ["Integer cents", "Cited figures", "15% VAT", "R5 000 and R50 thresholds", "REST and MCP", "Apache-2.0", "Self-hosted"];
  const group = (hidden) =>
    h("div", { class: "marquee-group", "aria-hidden": hidden ? "true" : null }, items.map((text) => h("span", {}, text)));
  return h("div", { class: "marquee", "aria-label": "What ClearCut is made of" },
    h("div", { class: "marquee-track" }, group(false), group(true)));
}

function sectionHead(eyebrow, title, sub) {
  return h("div", { class: "section-head" }, h("p", { class: "eyebrow" }, eyebrow), h("h2", {}, title), sub ? h("p", {}, sub) : null);
}

function stepsSection() {
  const steps = [
    ["Upload the invoice", "One PDF, PNG or JPEG. ClearCut keeps the file, the pages it read and the figures it took from each one."],
    ["ClearCut reads and checks", "Every figure is tied to the page it came from, then the report is checked against the source numbers before you see it."],
    ["You review and export", "Confirm anything the rules flag as uncertain, then export JSON, CSV, XLSX or PDF for your books."],
  ];
  const list = h("div", { class: "steps", id: "how" },
    steps.map(([title, copy], i) =>
      h("div", { class: "step" },
        h("span", { class: "step-num" }, String(i + 1).padStart(2, "0")),
        h("h3", {}, title),
        h("p", {}, copy))));
  return h("section", { class: "section" },
    sectionHead("How it works", "Three steps, and the source stays in view.", "ClearCut does not guess at an invoice. What it cannot read with confidence is handed to a person."),
    list);
}

function factsSection() {
  const facts = [
    [15, "%", "The standard VAT rate ClearCut applies, and states on every total."],
    [5000, "", "Above this, South African rules require a full tax invoice."],
    [50, "", "At or below this, a till slip is enough. The band between gets a simplified invoice."],
    [3, "", "File types read today: PDF, PNG and JPEG. Amounts stay integer cents end to end."],
  ];
  return h("section", { class: "section" },
    sectionHead("Built for South African invoicing", "Rules as tested code, not a footnote.", "The thresholds and rates come from the VAT Act and live in the same tests that check every total."),
    h("div", { class: "facts" },
      facts.map(([to, suffix, label]) => {
        const value = h("div", { class: "fact-value" });
        const prefix = suffix === "" && to >= 50 ? "R " : "";
        ticker(value, to, prefix, suffix);
        return h("div", { class: "fact" }, value, h("p", { class: "fact-label" }, label));
      })));
}

function agentSection() {
  return h("section", { class: "section" },
    h("div", { class: "panel-ink" },
      h("div", { class: "inner" },
        h("p", { class: "eyebrow" }, "Agent native"),
        h("h2", {}, "Built for agents, governed for finance."),
        h("p", {}, "The same Rust core answers a person in this page, a script over REST and an assistant over MCP. Agents can draft and calculate. Deleting a document, or anything a person must confirm, stays with a person."),
        h("ul", { class: "panel-grid" },
          h("li", {}, h("strong", {}, "REST API"), h("span", {}, "Uploads are idempotent: sending the same file twice gives one document. Typed errors, no silent rewrites.")),
          h("li", {}, h("strong", {}, "MCP server"), h("span", {}, "calculate_invoice, za_required_tax_invoice and check_status_transition, over stdio.")),
          h("li", {}, h("strong", {}, "Claude plugin"), h("span", {}, "The same tools announced to an assistant, so it can price an invoice without a browser.")))),
      h("pre", { class: "code" }, "curl -s https://your-clear-cut/v1/whoami \\\n  -H \"Authorization: Bearer $CLEARCUT_KEY\"\n\n", h("b", {}, "{ \"tenant\": \"...\", \"scopes\": [\"documents:read\", \"documents:write\"] }"))));
}

function openSection() {
  return h("section", { class: "section open" },
    h("div", {},
      h("p", { class: "eyebrow" }, "Open"),
      h("h2", {}, "Open, and yours to host."),
      h("p", { class: "lede" }, "ClearCut is Apache-2.0. The page loads nothing from anywhere else: no analytics, no third-party scripts, no fonts from a CDN. Your key stays in the page and your documents are read through your own Anthropic key.")),
    h("ul", { class: "license-list" },
      h("li", {}, h("b", {}, "Apache-2.0"), "Read it, run it, change it."),
      h("li", {}, h("b", {}, "One VM"), "Docker Compose on a single small server, with PostgreSQL beside it."),
      h("li", {}, h("b", {}, "Your data"), "Documents live on your disk, and deletion removes the file and everything read from it.")));
}

function trustPage() {
  stopPolling();
  setSignedIn(key !== null);
  const item = (label, ...text) => h("li", {}, h("b", {}, label), h("span", {}, ...text));
  show(h("div", { class: "home" },
    h("section", { class: "section" },
      h("div", { class: "section-head" },
        h("p", { class: "eyebrow" }, "Trust"),
        h("h2", {}, "Who runs ClearCut, and what it promises"),
        h("p", {}, "Plain statements about ownership, the licence, what happens to your documents and how to report a problem. Each one describes what this repository does. The formal terms and privacy notice are still drafts, waiting on review by someone admitted to practise in South Africa.")),
      h("ul", { class: "license-list" },
        item("Owner", "ClearCut is written and operated by Kone Tshivhinda. Copyright 2026 Kone Tshivhinda."),
        item("Licence", "Apache-2.0: you may read it, run it and change it. The ", h("a", { href: REPO + "/blob/main/LICENSE", rel: "noreferrer" }, "licence text"), " and the ", h("a", { href: REPO + "/blob/main/NOTICE", rel: "noreferrer" }, "third-party notices"), " travel with it."),
        item("Alpha", "ClearCut is in active development. Screens and behaviour change without notice, and this instance makes no uptime promise."),
        item("Warranty", "The software is provided as is, without warranties or conditions of any kind, as section 7 of the licence says. To the extent the law allows, nobody is liable for a loss caused by relying on it."),
        item("Not advice", "ClearCut reads documents and checks figures. It is not tax, legal or accounting advice, and it does not replace a person."),
        item("Figures", "A model can read a number wrongly, and a document can try to mislead it. Check every figure that matters against the original before you rely on it. Every reading cites its page, and an uncertain one waits for a person."),
        item("Security", "Report a vulnerability privately through ", h("a", { href: REPO + "/security/advisories/new", rel: "noreferrer" }, "GitHub Security Advisories"), ". Do not open a public issue. You should hear back within 5 working days, and only main is supported."))),
    h("section", { class: "section" },
      h("div", { class: "section-head" },
        h("p", { class: "eyebrow" }, "Your documents"),
        h("h2", {}, "What is stored, what leaves, and how it goes")),
      h("ul", { class: "license-list" },
        item("Storage", "Documents live on the server the operator runs, in a database and a folder only that server reaches. Each account's rows are separated by the database itself, so another account cannot read or change them."),
        item("Leaving", "Only the pages being read go to Anthropic, under the account's own key. Nothing goes anywhere else: the page loads no analytics, no third-party scripts and no fonts from a CDN. Bank and identity numbers are painted out of what is sent where the redaction policy finds them."),
        item("Anthropic", "Prompts and outputs are not kept by default for the models ClearCut uses. The top model, Fable 5.1, keeps them for 30 days, is never used for training without permission, and is off by default here."),
        item("Deletion", "Deleting a document erases the file and everything read from it. The audit entry stays, without content. This instance also erases documents after its retention period, 365 days by default."),
        item("Your key", "Your ClearCut key stays in this page's memory. There is no cookie, so reloading signs you out."),
        item("Who decides", "The person or organisation running this instance decides why documents are processed. ClearCut is software, not the responsible party; a request about a document goes to them."),
        item("More", "The repository carries the ", h("a", { href: REPO + "/blob/main/docs/THREAT-MODEL.md", rel: "noreferrer" }, "threat model"), " and the ", h("a", { href: REPO + "/blob/main/docs/privacy/RETENTION-AND-DELETION.md", rel: "noreferrer" }, "retention and deletion notes"), ".")))));
}

function home(message) {
  stopPolling();
  key = null;
  setSignedIn(false);
  const input = h("input", { type: "password", id: "key", autocomplete: "off", spellcheck: "false", required: true });
  const error = h("p", { class: "error small", role: "alert" }, message || "");
  const form = h(
    "form",
    {
      class: "card",
      onsubmit: async (event) => {
        event.preventDefault();
        key = input.value.trim();
        input.value = "";
        try {
          const response = await api("/v1/whoami");
          if (!response.ok) throw new AuthError("The key was not accepted.");
          setSignedIn(true);
          location.hash = "#/";
          route();
        } catch (e) {
          key = null;
          error.textContent = e.message || "Could not reach the server.";
        }
      },
    },
    h("h2", {}, "Sign in"),
    h("p", { class: "muted" }, "Paste your ClearCut key to open your documents."),
    h("label", { for: "key" }, "ClearCut key"),
    input,
    h("p", { class: "field-help" }, "The key stays in this page only. Reloading signs you out."),
    error,
    h("div", { class: "actions" }, h("button", { type: "submit" }, "Sign in")),
  );
  const hero = h("section", { class: "hero" },
    h("div", { class: "hero-inner" },
      h("div", {},
        h("span", { class: "chip" }, "Alpha · South African invoicing"),
        h("h1", {}, "Invoices in. ", h("em", {}, "Clear answers out.")),
        h("p", { class: "lede" }, "ClearCut reads a supplier invoice, checks every figure against the page it came from, and turns it into a record you can review, export and trust."),
        h("div", { class: "ctas" },
          h("a", { class: "button cta-glow", href: "#/demo" }, "Try the live demo"),
          h("a", { class: "button secondary", href: "#how" }, "See how it works"),
          h("a", { href: REPO, rel: "noreferrer" }, "Read the source")),
        h("p", { class: "cta-note shine" }, "Open source, Apache-2.0, and small enough to read.")),
      h("div", { class: "signin" }, form,
        h("div", { class: "trust-line" }, h("span", {}, "Integer cents"), h("span", {}, "Cited pages"), h("span", {}, "Your own Anthropic key")))));
  show(h("div", { class: "home" }, hero, marquee(), stepsSection(), factsSection(), agentSection(), openSection()));
}

function aboutPage() {
  stopPolling();
  setSignedIn(true);
  const hero = h("section", { class: "hero" },
    h("div", { class: "hero-inner" },
      h("div", {},
        h("span", { class: "chip" }, "Alpha · South African invoicing"),
        h("h1", {}, "Invoices in. ", h("em", {}, "Clear answers out.")),
        h("p", { class: "lede" }, "ClearCut reads a supplier invoice, checks every figure against the page it came from, and turns it into a record you can review, export and trust."),
        h("div", { class: "ctas" },
          h("a", { class: "button", href: "#/" }, "Open your documents"),
          h("a", { href: REPO, rel: "noreferrer" }, "Read the source"))),
      h("div", { class: "signin" },
        h("div", { class: "card" },
          h("h2", {}, "Where things are"),
          h("ul", { class: "license-list" },
            h("li", {}, h("b", {}, "Documents"), "Upload and follow what was read."),
            h("li", {}, h("b", {}, "Settings"), "Your Anthropic key and its limits."),
            h("li", {}, h("b", {}, "Report"), "Each document opens onto its own report."))))));
  show(h("div", { class: "home" }, hero, marquee(), stepsSection(), factsSection(), agentSection(), openSection()));
}

function signIn(message) {
  home(message);
}

signoutButton.addEventListener("click", () => {
  location.hash = "#/";
  home("");
});

exitDemoButton.addEventListener("click", () => {
  stopPolling();
  demoReset();
  location.hash = "#/";
});

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

// ---------------------------------------------------------------- bulk intake ----

// Several files, one after another through the same upload call. A failure on one does not stop
// the rest. The batch is capped because every document that is read is sent to Anthropic and
// costs money; the screen says so before anything is chosen.
function bulkCard(refresh) {
  const input = h("input", { type: "file", multiple: true,
    accept: ".pdf,.png,.jpg,.jpeg,application/pdf,image/png,image/jpeg" });
  const pick = h("label", { class: "drop" },
    h("strong", {}, "Choose up to 20 invoices"),
    " to upload one after another, or drop them here",
    input);
  const note = h("p", { class: "small muted" },
    "Each document is sent to Anthropic to be read and costs money. Files are uploaded one at a time; a failure does not stop the others.");
  const batchNote = h("p", { class: "small muted" }, "");
  const list = h("div", {});
  const rows = [];
  let running = false;

  function redraw() {
    list.replaceChildren(h("div", { class: "table-wrap" }, h("table", {},
      h("thead", {}, h("tr", {},
        h("th", {}, "File"), h("th", {}, "Status"), h("th", {}, "Open"))),
      h("tbody", {}, rows.map((row) => h("tr", {},
        h("td", {}, row.file.name), row.state, row.act))))));
  }

  function retry(row) {
    row.act.replaceChildren(h("button", { type: "button", class: "secondary",
      onclick: () => {
        row.act.replaceChildren();
        // A file whose upload succeeded is only asked about again; it is never sent twice.
        const again = row.documentId ? follow(row) : uploadOne(row);
        again.catch((e) => {
          if (e instanceof AuthError) {
            key = null;
            setSignedIn(false);
            signIn(e.message);
          }
        });
      } }, "Try again"));
  }

  async function follow(row) {
    row.state.textContent = "Reading…";
    for (let tries = 0; tries < 120; tries += 1) {
      let info;
      try {
        info = await json("/v1/documents/" + row.documentId);
      } catch (e) {
        if (e instanceof AuthError) throw e;
        row.state.textContent = e.message || "Could not check the document.";
        retry(row);
        return;
      }
      if (info.status === "done" || info.status === "needs_review") {
        row.state.replaceChildren(badge(info.status));
        row.act.replaceChildren(h("a", { href: "#/d/" + row.documentId }, "Open"));
        if (refresh) refresh();
        return;
      }
      if (info.status !== "received" && info.status !== "processing") {
        row.state.textContent = "Stopped (" + info.status + ")";
        retry(row);
        return;
      }
      await new Promise((done) => setTimeout(done, 3000));
    }
    row.state.textContent = "Still reading";
    row.act.replaceChildren(h("a", { href: "#/d/" + row.documentId }, "Open"));
    if (refresh) refresh();
  }

  async function uploadOne(row) {
    row.state.textContent = "Uploading…";
    try {
      const response = await api("/v1/documents?filename=" + encodeURIComponent(row.file.name), {
        method: "POST",
        headers: { "Content-Type": row.file.type || "application/pdf" },
        body: row.file,
      });
      const body = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(body.error || "The upload was refused (" + response.status + ").");
      row.documentId = body.document_id;
      await follow(row);
    } catch (e) {
      if (e instanceof AuthError) throw e;
      row.state.textContent = e.message || "The upload failed.";
      retry(row);
    }
  }

  async function run(files) {
    if (running) {
      batchNote.textContent = "A batch is already running. Wait for it to finish, then choose the rest.";
      return;
    }
    running = true;
    batchNote.textContent = "";
    const batch = files.slice(0, 20);
    try {
      if (files.length > 20) {
        batchNote.textContent = "Only the first 20 files were taken. Choose the rest in another batch.";
      }
      rows.length = 0;
      for (const file of batch) {
        rows.push({ file, documentId: null, state: h("td", {}, "Waiting"), act: h("td", {}) });
      }
      redraw();
      for (const row of rows) {
        try {
          await uploadOne(row);
        } catch (e) {
          if (e instanceof AuthError) {
            key = null;
            setSignedIn(false);
            return signIn(e.message);
          }
        }
      }
      if (refresh) refresh();
    } finally {
      running = false;
    }
  }

  input.addEventListener("change", () => {
    const files = [...input.files];
    input.value = "";
    if (files.length) run(files);
  });
  pick.addEventListener("dragover", (event) => { event.preventDefault(); pick.classList.add("over"); });
  pick.addEventListener("dragleave", () => pick.classList.remove("over"));
  pick.addEventListener("drop", (event) => {
    event.preventDefault();
    pick.classList.remove("over");
    if (event.dataTransfer.files.length) run([...event.dataTransfer.files]);
  });

  return h("div", {}, pick, note, batchNote, list);
}

// ---------------------------------------------------------------- 2. documents ----

async function documents() {
  stopPolling();
  const status = h("p", { class: "small muted", role: "status" }, "");
  const fileInput = h("input", { type: "file", accept: ".pdf,.png,.jpg,.jpeg,application/pdf,image/png,image/jpeg" });
  const chosen = new Set();
  const makeRegister = h("button", { type: "button", class: "secondary", disabled: true,
    onclick: () => {
      location.hash = "#/register?ids=" + [...chosen].join(",");
    } }, "Make a register from these");
  const drop = h(
    "label",
    { class: "drop" },
    h("strong", {}, "Choose an invoice"),
    " (PDF, PNG or JPEG) or drop it here",
    fileInput,
  );
  async function upload(file) {
    if (!file) return;
    status.textContent = "Uploading " + file.name + "…";
    status.className = "small muted";
    try {
      const response = await api("/v1/documents?filename=" + encodeURIComponent(file.name), {
        method: "POST",
        headers: { "Content-Type": file.type || "application/pdf" },
        body: file,
      });
      const body = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(body.error || "The upload was refused (" + response.status + ").");
      location.hash = "#/d/" + body.document_id;
    } catch (e) {
      if (e instanceof AuthError) return signIn(e.message);
      status.textContent = e.message;
      status.className = "small error";
    }
  }
  fileInput.addEventListener("change", () => upload(fileInput.files[0]));
  drop.addEventListener("dragover", (e) => { e.preventDefault(); drop.classList.add("over"); });
  drop.addEventListener("dragleave", () => drop.classList.remove("over"));
  drop.addEventListener("drop", (e) => {
    e.preventDefault();
    drop.classList.remove("over");
    upload(e.dataTransfer.files[0]);
  });

  const list = h("div", {}, h("div", { class: "states" }, "Loading…"));
  const bulk = bulkCard(load);
  show(h("div", { class: "page" },
    h("div", { class: "page-head" },
      h("p", { class: "eyebrow" }, "Workspace"),
      h("h1", {}, "Documents"),
      h("p", { class: "lede" }, "Upload an invoice to have it read. Everything shown here is checked against the page it came from.")),
    h("div", { class: "card" }, drop, status),
    h("div", { class: "card" },
      h("h2", {}, "Upload several"),
      bulk),
    h("div", { class: "row" }, makeRegister,
      h("p", { class: "small muted" }, "Tick documents to put them in one register.")),
    h("h2", {}, "Your documents"),
    list));

  async function load() {
    try {
      const { documents: rows } = await json("/v1/documents");
      if (rows.length === 0) {
        list.replaceChildren(h("div", { class: "states" }, "No documents yet. Upload an invoice above."));
      } else {
        list.replaceChildren(
          h(
            "div",
            { class: "table-wrap" },
            h(
              "table",
              {},
              h("thead", {}, h("tr", {}, h("th", {}, "Choose"), h("th", {}, "File"), h("th", {}, "Status"), h("th", { class: "num" }, "Pages"), h("th", {}, "Uploaded"))),
              h(
                "tbody",
                {},
                rows.map((d) =>
                  h(
                    "tr",
                    { class: "click", tabindex: "0", onclick: () => (location.hash = "#/d/" + d.id), onkeydown: (e) => { if (e.key === "Enter") location.hash = "#/d/" + d.id; } },
                    h("td", { class: "choose" }, h("input", { type: "checkbox", checked: chosen.has(d.id), "aria-label": "Choose " + d.original_filename,
                      onclick: (event) => {
                        event.stopPropagation();
                        if (event.currentTarget.checked) chosen.add(d.id);
                        else chosen.delete(d.id);
                        makeRegister.disabled = chosen.size === 0;
                      },
                      onkeydown: (event) => { if (event.key === "Enter") event.stopPropagation(); } })),
                    h("td", {}, d.original_filename),
                    h("td", {}, badge(d.status)),
                    h("td", { class: "num" }, d.page_count ?? "–"),
                    h("td", {}, when(d.created_at)),
                  ),
                ),
              ),
            ),
          ),
        );
      }
      makeRegister.disabled = chosen.size === 0;
      if (rows.some((d) => d.status === "received" || d.status === "processing")) timer = setTimeout(load, 3000);
    } catch (e) {
      const shown = failure(e, load);
      if (shown instanceof Node) list.replaceChildren(shown);
    }
  }
  load();
}

// ---------------------------------------------------------------- 3. one document ----

async function documentPage(id) {
  stopPolling();
  const body = h("div", {}, h("div", { class: "states" }, "Loading…"));
  show(h("div", { class: "page" }, h("p", { class: "small" }, h("a", { href: "#/" }, "← All documents")), body));
  let mode = "summary";
  let finishing = null; // the job queued by a confirmation, until it has ended

  async function draw() {
    let info, jobs, events;
    try {
      [info, jobs, events] = await Promise.all([
        json("/v1/documents/" + id),
        json("/v1/documents/" + id + "/jobs"),
        json("/v1/documents/" + id + "/events"),
      ]);
    } catch (e) {
      const shown = failure(e, draw);
      if (shown instanceof Node) body.replaceChildren(shown);
      return;
    }
    const job = (jobs.jobs || [])[0];
    const mine = finishing && (jobs.jobs || []).find((j) => j.id === finishing);
    if (mine && !["queued", "running"].includes(mine.state)) finishing = null;
    const working = info.status === "received" || info.status === "processing" || Boolean(finishing);
    let waiting = [];
    if (info.status === "needs_review" && !finishing) {
      try {
        waiting = (await json("/v1/documents/" + id + "/review")).pages;
      } catch (e) {
        if (e instanceof AuthError) return signIn(e.message);
      }
    }
    const waitingForKey = Boolean(job && job.state === "queued" && job.last_error && /Anthropic key/.test(job.last_error));
    const parts = [
      h("div", { class: "row" }, h("h1", {}, info.original_filename), badge(info.status)),
      h("p", { class: "muted small" }, info.page_count ? info.page_count + (info.page_count === 1 ? " page" : " pages") : "Pages not counted yet"),
    ];
    if (working) {
      const p = job && job.progress;
      parts.push(
        h(
          "div",
          { class: "card", role: "status" },
          waitingForKey
            ? h("p", {}, h("strong", {}, "Waiting for your Anthropic key. "), "Add it in ", h("a", { href: "#/settings" }, "Settings"), "; this document starts within a minute.")
            : h("p", {}, job && job.progress_note ? "Working: " + job.progress_note : "Waiting to be read…"),
          waitingForKey ? null : p && p.total ? h("progress", { max: p.total, value: p.done }) : h("progress", {}),
          waitingForKey ? null : p && p.total ? h("p", { class: "small muted" }, "Step " + p.done + " of " + p.total) : null,
        ),
      );
    }
    if (info.status === "needs_review") {
      parts.push(
        h(
          "div",
          { class: "banner", role: "note" },
          h("strong", {}, "A person needs to look at this document. "),
          job && job.last_error ? job.last_error + "." : "",
          h("br"),
          h("span", { class: "small muted" }, "What was read is below; check it against the original before relying on it."),
        ),
      );
    }
    if (waiting.length > 0) {
      parts.push(reviewPanel(id, waiting, (jobId) => { finishing = jobId; draw(); }));
    }
    if (info.events > 0) {
      parts.push(reportSection(), tableSection(events.table), downloads());
    } else if (!working) {
      parts.push(h("div", { class: "states" }, "Nothing was read from this document."));
    }
    if (!working) parts.push(eraseSection());
    body.replaceChildren(...parts);
    if (info.events > 0) loadReport();
    if (working) timer = setTimeout(draw, 2000);
  }

  const reportBox = h("div", { class: "report" }, h("p", { class: "muted" }, "Loading…"));
  function reportSection() {
    const tabs = h("div", { class: "tabs", role: "tablist" });
    for (const [value, label] of [["summary", "Summary"], ["timeline", "Timeline"], ["grouped", "By group"]]) {
      tabs.append(
        h("button", {
          type: "button", role: "tab", "aria-selected": String(mode === value),
          onclick: (ev) => { mode = value; for (const b of tabs.children) b.setAttribute("aria-selected", String(b === ev.currentTarget)); loadReport(); },
        }, label),
      );
    }
    return h("div", {}, h("h2", {}, "Report"), tabs, reportBox);
  }
  async function loadReport() {
    try {
      const report = await json("/v1/documents/" + id + "/report?mode=" + mode);
      const sections = report.sections.map((s) =>
        h(
          "div",
          {},
          s.heading ? h("h3", {}, s.heading) : null,
          h("ul", {}, s.sentences.map((x) => h("li", {}, x.text, h("span", { class: "cite" }, x.cites.map((c) => c.split(":").pop()).join(", "))))),
        ),
      );
      reportBox.replaceChildren(...(sections.length ? sections : [h("p", { class: "muted" }, "Nothing to report.")]));
    } catch (e) {
      const shown = failure(e, loadReport);
      if (shown instanceof Node) reportBox.replaceChildren(shown);
    }
  }

  function tableSection(rows) {
    return h(
      "div",
      {},
      h("h2", {}, "Everything that was read"),
      h(
        "div",
        { class: "table-wrap" },
        h(
          "table",
          {},
          h("thead", {}, h("tr", {}, h("th", {}, "What"), h("th", {}, "Statement"), h("th", { class: "num" }, "Page"), h("th", { class: "num" }, "Sure"))),
          h(
            "tbody",
            {},
            rows.map((r) =>
              h(
                "tr",
                { class: r.severity },
                h("td", {}, r.group + " · " + r.type.replaceAll("_", " ")),
                h("td", {}, r.text),
                h("td", { class: "num" }, r.page),
                h("td", { class: "num" }, Math.round(r.confidence_per_mille / 10) + "%"),
              ),
            ),
          ),
        ),
      ),
    );
  }

  function eraseSection() {
    const message = h("p", { class: "small error", role: "alert" });
    const button = h(
      "button",
      {
        type: "button",
        class: "secondary danger",
        onclick: async () => {
          if (!confirm("Delete this document, what was read from it and its file for good? This cannot be undone.")) return;
          button.disabled = true;
          try {
            const response = await api("/v1/documents/" + id, { method: "DELETE" });
            if (response.status === 409) throw new Error("The document is being worked on. Try again when it has finished.");
            if (!response.ok && response.status !== 404) throw new Error("The document was not deleted (" + response.status + ").");
            location.hash = "#/";
          } catch (e) {
            if (e instanceof AuthError) return signIn(e.message);
            message.textContent = e.message;
            button.disabled = false;
          }
        },
      },
      "Delete this document",
    );
    return h("div", {}, h("h2", {}, "Delete"), h("p", { class: "small muted" }, "Removes the document, everything read from it and its file. A record that it was deleted (with no content) is kept."), button, message);
  }

  function downloads() {
    const make = (format, label) =>
      h("button", {
        type: "button", class: "secondary",
        onclick: async (e) => {
          const button = e.currentTarget;
          button.disabled = true;
          try {
            const response = await api("/v1/documents/" + id + "/export?format=" + format);
            if (!response.ok) throw new Error("The download failed (" + response.status + ").");
            const url = URL.createObjectURL(await response.blob());
            const link = h("a", { href: url, download: "clearcut-" + id + "." + format });
            document.body.append(link);
            link.click();
            link.remove();
            setTimeout(() => URL.revokeObjectURL(url), 10000);
          } catch (err) {
            alert(err.message);
          } finally {
            button.disabled = false;
          }
        },
      }, label);
    return h(
      "div",
      {},
      h("h2", {}, "Download"),
      h("div", { class: "row" },
        make("pdf", "PDF report"),
        make("xlsx", "Excel workbook"),
        make("docx", "Word document"),
        make("html", "Web page"),
        make("txt", "Plain text"),
        make("csv", "CSV (opens in Excel)"),
        make("md", "Markdown"),
        make("json", "JSON"),
      ),
    );
  }

  draw();
}

// ---------------------------------------------------------------- review ----
// A page the reader could not settle: the person sees what was read (or a blank form), corrects it
// and confirms. The server checks it with the same rules as a model's reading and says exactly what
// is still wrong. Everything here is text from the document, so it only ever goes in through
// value/textContent.

const REASONS = {
  tax_not_shown: "The page does not say how its lines are taxed. Choose the tax category of each line, as the original shows it. If the original does not say, leave this for someone who can find out.",
  top_rung_failed: "The reader could not produce a reading of this page that passes ClearCut's checks.",
  too_long: "This page has too much on it to be read in one go.",
  refused: "The model declined to read this page.",
  budget: "Reading stopped to protect your spending limit.",
  call_cap: "Reading stopped because the page or the document used up its allowed calls.",
  cannot_be_sent_safely: "The page could not be sent to the model safely, so nothing was sent.",
  request_rejected: "The model service rejected the request. This is a fault in ClearCut.",
  no_rung_available: "Your key can use none of the models ClearCut tries.",
  amounts_include_vat: "The amounts on this page include VAT. ClearCut divided the standard-rated lines by 1.15 to show them without VAT, so check a line or two against the original.",
};

const KINDS = {
  bad_amount: "is not an exact amount (digits, with a dot for cents)",
  bad_quantity: "is not a quantity (digits, at most three decimals)",
  bad_date: "is not a date (write it as YYYY-MM-DD)",
  bad_characters: "has characters that are not allowed",
  too_long: "is too long",
  too_many_lines: "has too many lines for one page",
  missing_required: "is missing",
  tax_not_shown: "is not shown on the page",
  incomplete: "is only partly filled in",
  amounts_include_vat: "includes VAT, which was divided out",
  not_on_page: "was not found on the page",
};

const FIELD_NAMES = {
  description: "description", quantity: "quantity", unit_price: "unit price", line_total: "line total", tax: "tax category",
  net: "net", vat: "VAT", gross: "gross", invoice_number: "invoice number", invoice_date: "invoice date",
};

function placeText(path) {
  const line = path.match(/^lines\/(\d+)\/(\w+)$/);
  if (line) return "Line " + line[1] + ", " + (FIELD_NAMES[line[2]] || line[2]);
  const totals = path.match(/^totals\/(\w+)$/);
  if (totals) return "Totals, " + (FIELD_NAMES[totals[1]] || totals[1]);
  return FIELD_NAMES[path] || path.replaceAll("/", ", ").replaceAll("_", " ");
}

function moneyText(m) {
  const minor = Number(m.minor);
  const abs = Math.abs(minor);
  return (minor < 0 ? "-" : "") + m.currency + " " + Math.trunc(abs / 100) + "." + String(abs % 100).padStart(2, "0");
}

function findingText(f) {
  switch (f.type) {
    case "no_lines": return "No lines were entered.";
    case "line_total_mismatch": return "Line " + f.line + ": quantity times unit price is " + moneyText(f.recomputed) + ", but the line total says " + moneyText(f.stated) + ".";
    case "totals_mismatch": return "The " + (FIELD_NAMES[f.field] || f.field) + " total says " + moneyText(f.stated) + ", but the lines work out to " + moneyText(f.recomputed) + ".";
    case "non_positive_quantity": return "Line " + f.line + ": the quantity must be more than zero.";
    case "amount_out_of_range": return "Line " + f.line + ": an amount is too large.";
    case "currency_mismatch": return "Line " + f.line + ": the amounts are in a different currency.";
    case "totals_out_of_range": return "The totals are too large to add up.";
    default: return "A check failed.";
  }
}

const TAX_CHOICES = [
  ["not_shown", "Not shown"], ["standard", "Standard (15%)"], ["zero_rated", "Zero-rated"], ["exempt", "Exempt"], ["out_of_scope", "Outside the scope of VAT"],
];

function party(p) {
  return { name: (p && p.name) || "", address: (p && p.address) || "", vat_number: (p && p.vat_number) || "" };
}

// What the model read (or nothing), with every field present.
function normalise(draft) {
  const d = draft || {};
  return {
    currency: d.currency || "not_shown",
    amounts_include_vat: d.amounts_include_vat || "not_shown",
    invoice_number: d.invoice_number || "",
    invoice_date: d.invoice_date || "",
    supplier: party(d.supplier),
    recipient: party(d.recipient),
    lines: (d.lines || []).map((l) => ({
      description: l.description || "", quantity: l.quantity || "", unit_price: l.unit_price || "",
      line_total: l.line_total || "", tax: l.tax || "not_shown",
    })),
    totals: { net: (d.totals && d.totals.net) || "", vat: (d.totals && d.totals.vat) || "", gross: (d.totals && d.totals.gross) || "" },
    carried: {
      brought_forward: (d.carried && d.carried.brought_forward) || "",
      carried_forward: (d.carried && d.carried.carried_forward) || "",
    },
    due: { date: (d.due && d.due.date) || "", terms: (d.due && d.due.terms) || "" },
    credit_note: {
      original_invoice_number: (d.credit_note && d.credit_note.original_invoice_number) || "",
      amount: (d.credit_note && d.credit_note.amount) || "",
    },
  };
}

let fieldCounter = 0;

function textField(label, obj, key) {
  const id = "f" + ++fieldCounter;
  const input = h("input", { type: "text", id, value: obj[key], autocomplete: "off", spellcheck: "false", oninput: () => { obj[key] = input.value; } });
  return h("div", { class: "field" }, h("label", { for: id }, label), input);
}

function choiceField(label, obj, key, choices) {
  const id = "f" + ++fieldCounter;
  const select = h("select", { id, onchange: () => { obj[key] = select.value; } }, choices.map(([value, text]) => h("option", { value, selected: obj[key] === value }, text)));
  return h("div", { class: "field" }, h("label", { for: id }, label), select);
}

function cell(obj, key, label) {
  const input = h("input", { type: "text", value: obj[key], autocomplete: "off", spellcheck: "false", "aria-label": label, oninput: () => { obj[key] = input.value; } });
  return h("td", {}, input);
}

function reviewPanel(id, waiting, saved) {
  return h(
    "div",
    {},
    h("h2", {}, "Needs your review"),
    h("p", { class: "muted" }, "ClearCut could not settle these pages on its own. Check each one against the original, correct anything that is wrong and confirm it. Your confirmation is recorded against your key. ClearCut still checks the arithmetic and will not accept figures that do not add up."),
    waiting.map((page) => pageForm(id, page, saved)),
  );
}

function pageForm(id, waiting, saved) {
  const r = normalise(waiting.reading);
  const messages = h("div", { role: "alert" });
  const linesBox = h("div", { class: "table-wrap" });

  function drawLines() {
    linesBox.replaceChildren(
      h(
        "table",
        {},
        h("thead", {}, h("tr", {}, ["Description", "Quantity", "Unit price", "Line total", "Tax category", ""].map((t) => h("th", {}, t)))),
        h(
          "tbody",
          {},
          r.lines.map((line, index) =>
            h(
              "tr",
              {},
              cell(line, "description", "Line " + (index + 1) + " description"),
              cell(line, "quantity", "Line " + (index + 1) + " quantity"),
              cell(line, "unit_price", "Line " + (index + 1) + " unit price"),
              cell(line, "line_total", "Line " + (index + 1) + " line total"),
              h("td", {}, (() => {
                const select = h("select", { "aria-label": "Line " + (index + 1) + " tax category", onchange: () => { line.tax = select.value; } },
                  TAX_CHOICES.map(([value, text]) => h("option", { value, selected: line.tax === value }, text)));
                return select;
              })()),
              h("td", {}, h("button", { type: "button", class: "link", onclick: () => { r.lines.splice(index, 1); drawLines(); } }, "Remove")),
            ),
          ),
        ),
      ),
    );
  }
  drawLines();

  const reasonText = REASONS[waiting.reason] || "This page needs a person to look at it.";
  const problems = (waiting.problems || []).map((p) => placeText(p.path) + " " + (KINDS[p.kind] || "could not be read") + ".");
  const button = h("button", { type: "button" }, "Confirm this page");
  button.addEventListener("click", async () => {
    button.disabled = true;
    messages.replaceChildren(h("p", { class: "muted small", role: "status" }, "Checking…"));
    try {
      const response = await api("/v1/documents/" + id + "/pages/" + waiting.page + "/review", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(r),
      });
      const body = await response.json().catch(() => ({}));
      if (response.status === 422 && body.refusal) {
        const list = [];
        if (body.refusal.currency_missing) list.push("Choose the currency of the page.");
        for (const p of body.refusal.problems) list.push(placeText(p.path) + " " + (KINDS[p.kind] || "could not be read") + ".");
        for (const f of body.refusal.findings) list.push(findingText(f));
        messages.replaceChildren(h("p", { class: "error" }, "This page cannot be confirmed yet:"), h("ul", {}, list.map((t) => h("li", {}, t))));
        button.disabled = false;
        return;
      }
      if (!response.ok) throw new Error(body.error || "The page was not saved (" + response.status + ").");
      messages.replaceChildren(h("p", { class: "muted small", role: "status" }, "Saved. Finishing the document…"));
      saved(body.job_id);
    } catch (e) {
      if (e instanceof AuthError) return signIn(e.message);
      messages.replaceChildren(h("p", { class: "error" }, e.message));
      button.disabled = false;
    }
  });

  return h(
    "div",
    { class: "card review" },
    h("h3", {}, "Page " + waiting.page),
    h("p", {}, reasonText),
    problems.length ? h("ul", { class: "small" }, problems.map((t) => h("li", {}, t))) : null,
    waiting.reading ? h("p", { class: "small muted" }, "The form shows what the reader made of the page, which may be wrong.") : h("p", { class: "small muted" }, "Nothing could be read from this page, so the form is blank."),
    h("div", { class: "form-grid" },
      choiceField("Currency", r, "currency", [["not_shown", "Not shown"], ["ZAR", "ZAR (rand)"], ["USD", "USD"], ["EUR", "EUR"], ["GBP", "GBP"]]),
      choiceField("Prices include VAT", r, "amounts_include_vat", [["not_shown", "Not shown"], ["no", "No"], ["yes", "Yes"]]),
      textField("Invoice number", r, "invoice_number"),
      textField("Invoice date (YYYY-MM-DD)", r, "invoice_date"),
      textField("Supplier name", r.supplier, "name"),
      textField("Supplier VAT number", r.supplier, "vat_number"),
      textField("Supplier address", r.supplier, "address"),
      textField("Recipient name", r.recipient, "name"),
      textField("Recipient VAT number", r.recipient, "vat_number"),
      textField("Recipient address", r.recipient, "address"),
      textField("Due date (YYYY-MM-DD)", r.due, "date"),
      textField("Payment terms", r.due, "terms"),
    ),
    h("h3", {}, "Lines"),
    linesBox,
    h("p", {}, h("button", { type: "button", class: "secondary", onclick: () => { r.lines.push({ description: "", quantity: "1", unit_price: "", line_total: "", tax: "not_shown" }); drawLines(); } }, "Add a line")),
    h("h3", {}, "Totals as printed"),
    h("p", { class: "small muted" }, "Fill in all three or none."),
    h("div", { class: "form-grid" }, textField("Net (before VAT)", r.totals, "net"), textField("VAT", r.totals, "vat"), textField("Gross (total)", r.totals, "gross")),
    h("h3", {}, "Carried on this page"),
    h("p", { class: "small muted" }, "For a long invoice: the amount brought forward at the top and the amount carried forward at the foot. Leave empty if not printed."),
    h("div", { class: "form-grid" }, textField("Brought forward", r.carried, "brought_forward"), textField("Carried forward", r.carried, "carried_forward")),
    h("h3", {}, "Credit note"),
    h("div", { class: "form-grid" }, textField("Original invoice number", r.credit_note, "original_invoice_number"), textField("Amount", r.credit_note, "amount")),
    h("div", { class: "row actions" }, button),
    messages,
  );
}

// ---------------------------------------------------------------- settings: your Anthropic key ----
// ClearCut has no Anthropic key of its own: each account brings its own (ADR-0019). It is sent once
// in the body of a PUT, kept encrypted on the server, and never shown again: only its last four
// characters. This page never keeps it.

async function settingsPage() {
  stopPolling();
  const body = h("div", {}, h("div", { class: "states" }, "Loading…"));
  show(h("div", { class: "page" },
    h("p", { class: "small" }, h("a", { href: "#/" }, "← All documents")),
    h("div", { class: "page-head" },
      h("p", { class: "eyebrow" }, "Workspace"),
      h("h1", {}, "Your Anthropic key"),
      h("p", { class: "lede" }, "ClearCut reads your documents with a key you own, set to a limit you choose.")),
    body));

  async function draw(savedNote) {
    let view;
    try {
      view = await json("/v1/anthropic-key");
    } catch (e) {
      const shown = failure(e, draw);
      if (shown instanceof Node) body.replaceChildren(shown);
      return;
    }
    const input = h("input", { type: "password", id: "anthropic-key", autocomplete: "off", spellcheck: "false" });
    const message = h("p", { class: "small", role: "status" });
    const save = h("button", { type: "button" }, view.configured ? "Replace key" : "Save key");
    save.addEventListener("click", async () => {
      const text = input.value.trim();
      input.value = "";
      if (!text) { message.className = "small error"; message.textContent = "Paste your key first."; return; }
      save.disabled = true;
      message.className = "small muted";
      message.textContent = "Checking the key with Anthropic (this costs nothing)…";
      try {
        const response = await api("/v1/anthropic-key", { method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ key: text }) });
        const result = await response.json().catch(() => ({}));
        if (!response.ok) throw new Error(result.error || "The key was not saved (" + response.status + ").");
        await draw("Saved. This key can use: " + (result.models.length ? result.models.join(", ") : "no models ClearCut knows") + ".");
      } catch (e) {
        if (e instanceof AuthError) return signIn(e.message);
        message.className = "small error";
        message.textContent = e.message;
        save.disabled = false;
      }
    });

    let status;
    if (!view.tenant_keys_enabled) {
      status = view.source === "server"
        ? "This server cannot store a key for you. Your documents are read with the server's own key."
        : "This server cannot store a key for you, and has none of its own, so documents are not read by a model.";
    } else if (view.configured && view.check_ok === false) {
      status = "A key ending " + view.last4 + " is saved, but Anthropic refused it. Paste a new one.";
    } else if (view.configured) {
      status = "A key ending " + view.last4 + " is saved" + (view.checked_at ? " (checked " + when(view.checked_at) + ")" : "") + ". It cannot be shown again.";
    } else if (view.source === "server") {
      status = "No key saved. Until you add one, documents are read with the server's own key.";
    } else {
      status = "No key saved. Documents wait until you add your own.";
    }

    const remove = view.configured
      ? h("button", {
          type: "button", class: "secondary danger",
          onclick: async (e) => {
            if (!confirm("Remove your saved Anthropic key? Documents will wait until you add one again.")) return;
            const button = e.currentTarget;
            button.disabled = true;
            try {
              const response = await api("/v1/anthropic-key", { method: "DELETE" });
              if (!response.ok) throw new Error("The key was not removed (" + response.status + ").");
              await draw();
            } catch (err) {
              if (err instanceof AuthError) return signIn(err.message);
              alert(err.message);
              button.disabled = false;
            }
          },
        }, "Remove my key")
      : null;

    body.replaceChildren(
      h("div", { class: "card" },
        h("p", {}, status),
        savedNote ? h("p", { class: "small" }, savedNote) : null,
        view.tenant_keys_enabled
          ? h("div", {},
              h("div", { class: "field" }, h("label", { for: "anthropic-key" }, view.configured ? "New Anthropic key" : "Anthropic key"), input),
              h("div", { class: "row" }, save, remove),
              message)
          : null),
      h("h2", {}, "Before you paste a key"),
      h("ul", { class: "small" },
        h("li", {}, "Every page read is billed to this key, so it is your money. Use a dedicated key made for ClearCut in the Anthropic Console."),
        h("li", {}, "Set a monthly spend limit on it there, and an expiry if you can. ClearCut also stops at R5 a document and R100 a month, but your own limit is the one that cannot be bypassed."),
        h("li", {}, "ClearCut stores it encrypted, uses it only to read your documents, never shows it again and never sends it anywhere except Anthropic. You can remove it here at any time."),
        h("li", {}, "Only a regular API key works. An admin key or a subscription login token is refused."),
        h("li", {}, "Your documents are sent to Anthropic, outside South Africa, to be read. Do not upload other people's personal information unless you are entitled to.")),
    );
  }
  draw();
}

// ---------------------------------------------------------------- 4. register ----

const PERIODS = [
  ["last", "Last month"],
  ["this", "This month"],
  ["quarter", "This quarter"],
  ["custom", "Custom range"],
];

const REGISTER_KINDS = { invoice: "Invoice", credit_note: "Credit note", unclassified: "Unclassified" };

const REGISTER_TAX = {
  standard: "Standard",
  zero_rated: "Zero-rated",
  exempt: "Exempt",
  out_of_scope: "Outside scope",
};

function isoDate(date) {
  return date.getFullYear() + "-" + String(date.getMonth() + 1).padStart(2, "0") + "-" +
    String(date.getDate()).padStart(2, "0");
}

// The first and last day of the chosen period, as YYYY-MM-DD. The local calendar decides what
// "last month" is; the server is given plain dates and does no time-zone arithmetic.
function periodBounds(choice, from, to) {
  const now = new Date();
  if (choice === "custom") return [from, to];
  if (choice === "this") {
    return [isoDate(new Date(now.getFullYear(), now.getMonth(), 1)),
      isoDate(new Date(now.getFullYear(), now.getMonth() + 1, 0))];
  }
  if (choice === "quarter") {
    const first = Math.floor(now.getMonth() / 3) * 3;
    return [isoDate(new Date(now.getFullYear(), first, 1)),
      isoDate(new Date(now.getFullYear(), first + 3, 0))];
  }
  return [isoDate(new Date(now.getFullYear(), now.getMonth() - 1, 1)),
    isoDate(new Date(now.getFullYear(), now.getMonth(), 0))];
}

function amountText(amount) {
  return amount ? amount.text : "–";
}

function rateText(bps) {
  const percent = bps / 100;
  return (Number.isInteger(percent) ? String(percent) : percent.toFixed(1)) + "%";
}

function previewParts(view) {
  const m = view.manifest;
  const parts = [];
  parts.push(h("div", { class: "card" },
    h("h2", {}, "This register"),
    h("p", {}, m.documents + " document(s): " + m.done + " done, " + m.needs_review +
      " awaiting review, " + m.still_reading + " still reading."),
    h("p", { class: "small muted" },
      m.automation_rate_bps == null
        ? "No finished documents yet, so there is no automation rate."
        : rateText(m.automation_rate_bps) + " of finished documents needed nobody.",
      " ",
      m.undated_in_selection
        ? m.undated_in_selection + " document(s) have no date and are not in a month. "
        : "",
      m.awaiting_left_out
        ? m.awaiting_left_out + " awaiting review were left out."
        : "")));

  if (view.rows.length === 0) {
    parts.push(h("div", { class: "states" }, "No documents in this register. Change the period, or upload an invoice."));
  } else {
    const shown = view.rows.slice(0, 200);
    parts.push(h("h2", {}, "Documents"));
    parts.push(h("div", { class: "table-wrap" }, h("table", {},
      h("thead", {}, h("tr", {},
        h("th", {}, "Date"), h("th", {}, "Supplier"), h("th", {}, "Number"),
        h("th", {}, "Kind"), h("th", {}, "Status"), h("th", {}, "Currency"),
        h("th", { class: "num" }, "Net"), h("th", { class: "num" }, "VAT"),
        h("th", { class: "num" }, "Gross"), h("th", { class: "num" }, "Lines"))),
      h("tbody", {}, shown.map((row) => h("tr", {},
        h("td", {}, row.invoice_date || "No date"),
        h("td", {}, row.supplier || "–"),
        h("td", {}, row.invoice_number || "–"),
        h("td", {}, REGISTER_KINDS[row.kind] || row.kind),
        h("td", {}, badge(row.status)),
        h("td", {}, row.currency || "–"),
        h("td", { class: "num" }, amountText(row.net)),
        h("td", { class: "num" }, amountText(row.vat)),
        h("td", { class: "num" }, amountText(row.gross)),
        h("td", { class: "num" }, row.line_count)))))));
    if (view.rows.length > shown.length) {
      parts.push(h("p", { class: "small muted" },
        "Showing the first " + shown.length + " of " + view.rows.length + " documents. The file holds all of them."));
    }
  }

  if (view.vat_by_category.length) {
    const lines = [];
    for (const group of view.vat_by_category) {
      for (const [side, part] of [["Ready", group.ready], ["Awaiting review", group.awaiting]]) {
        if (!part) continue;
        lines.push(h("tr", {},
          h("td", {}, group.month), h("td", {}, group.currency),
          h("td", {}, REGISTER_KINDS[group.kind] || group.kind),
          h("td", {}, REGISTER_TAX[group.category] || group.category),
          h("td", { class: "num" }, rateText(group.rate_bps)),
          h("td", {}, side),
          h("td", { class: "num" }, part.documents),
          h("td", { class: "num" }, part.taxable.text),
          h("td", { class: "num" }, part.vat.text)));
      }
    }
    parts.push(h("h2", {}, "VAT by category"));
    parts.push(h("div", { class: "table-wrap" }, h("table", {},
      h("thead", {}, h("tr", {},
        h("th", {}, "Month"), h("th", {}, "Currency"), h("th", {}, "Kind"),
        h("th", {}, "Category"), h("th", { class: "num" }, "Rate"), h("th", {}, "Side"),
        h("th", { class: "num" }, "Documents"), h("th", { class: "num" }, "Taxable"),
        h("th", { class: "num" }, "VAT"))),
      h("tbody", {}, lines))));
  }

  if (view.reconciliation.length) {
    parts.push(h("h2", {}, "Calculated against stated"));
    parts.push(h("div", { class: "table-wrap" }, h("table", {},
      h("thead", {}, h("tr", {},
        h("th", {}, "Month"), h("th", {}, "Currency"), h("th", {}, "Kind"),
        h("th", { class: "num" }, "Calculated VAT"), h("th", { class: "num" }, "Stated VAT"),
        h("th", { class: "num" }, "Difference"), h("th", { class: "num" }, "Documents that disagree"))),
      h("tbody", {}, view.reconciliation.map((row) => h("tr", {},
        h("td", {}, row.month), h("td", {}, row.currency),
        h("td", {}, REGISTER_KINDS[row.kind] || row.kind),
        h("td", { class: "num" }, row.calculated_vat.text),
        h("td", { class: "num" }, row.stated_vat.text),
        h("td", { class: "num" }, row.difference.text),
        h("td", { class: "num" }, row.documents_disagree)))))));
  }

  parts.push(h("h2", {}, "To check"));
  if (view.to_check.length === 0) {
    parts.push(h("p", { class: "small muted" }, "Nothing is waiting for a person."));
  } else {
    parts.push(h("div", { class: "table-wrap" }, h("table", {},
      h("thead", {}, h("tr", {},
        h("th", {}, "Document"), h("th", {}, "Reason"), h("th", { class: "num" }, "Page"),
        h("th", {}, "What to check"))),
      h("tbody", {}, view.to_check.map((row) => h("tr", {},
        h("td", {}, row.filename),
        h("td", {}, row.reason),
        h("td", { class: "num" }, row.page ?? "–"),
        h("td", {}, row.sentence)))))));
  }

  parts.push(h("p", { class: "small muted" }, view.notice));
  return parts;
}

function registerPage(chosenIds) {
  stopPolling();
  const ids = (chosenIds || "").split(",").map((id) => id.trim()).filter(Boolean);

  const choice = h("select", { id: "register-period" },
    PERIODS.map(([value, label]) => h("option", { value }, label)));
  choice.value = "last";
  const from = h("input", { type: "date", id: "register-from" });
  const to = h("input", { type: "date", id: "register-to" });
  const [firstFrom, firstTo] = periodBounds("last", "", "");
  from.value = firstFrom;
  to.value = firstTo;
  const custom = h("span", { class: "row", hidden: true },
    h("label", { for: "register-from" }, "From"), from,
    h("label", { for: "register-to" }, "To"), to);
  const periodRow = h("div", { class: "row", hidden: ids.length > 0 },
    h("label", { for: "register-period" }, "Period"), choice, custom);
  choice.addEventListener("change", () => {
    custom.hidden = choice.value !== "custom";
    previewRegister();
  });
  from.addEventListener("change", previewRegister);
  to.addEventListener("change", previewRegister);
  const chosenNote = h("p", { class: "small muted" },
    ids.length + (ids.length === 1 ? " chosen document." : " chosen documents."));

  const status = h("p", { class: "small muted", role: "status" }, "");
  const preview = h("div", {}, h("div", { class: "states" },
    "The file is built when you ask for it; nothing is kept."));

  function request(format) {
    if (ids.length) return { document_ids: ids, format };
    const [start, end] = periodBounds(choice.value, from.value, to.value);
    if (!start || !end) throw new Error("Choose both dates.");
    return { from: start, to: end, format };
  }

  function ask(format) {
    return api("/v1/register", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(request(format)),
    });
  }

  async function previewRegister() {
    status.className = "small muted";
    status.textContent = "Building the preview…";
    try {
      const response = await ask("json");
      const view = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(view.error || "The register could not be built (" + response.status + ").");
      preview.replaceChildren(...previewParts(view));
      status.textContent = "";
    } catch (e) {
      if (e instanceof AuthError) return signIn(e.message);
      status.className = "small error";
      status.textContent = e.message || "Something went wrong.";
    }
  }

  async function download(format, label) {
    status.className = "small muted";
    status.textContent = "Making the " + label + "…";
    try {
      await saveFile(await ask(format), "clearcut-register." + (format === "csv" ? "zip" : format));
      status.textContent = "";
    } catch (e) {
      if (e instanceof AuthError) return signIn(e.message);
      status.className = "small error";
      status.textContent = e.message || "Something went wrong.";
    }
  }

  const button = (format, label) => h("button", {
    type: "button", class: format === "xlsx" ? "" : "secondary",
    onclick: () => download(format, label),
  }, label);

  show(h("div", { class: "page" },
    h("div", { class: "page-head" },
      h("p", { class: "eyebrow" }, "Workspace"),
      h("h1", {}, "Register"),
      h("p", { class: "lede" }, "Every document in a period, with its figures, its VAT and the things that need a person. Check the file against the originals before you use it.")),
    h("div", { class: "card" },
      ids.length ? chosenNote : null,
      periodRow,
      h("div", { class: "row" },
        h("button", { type: "button", onclick: previewRegister }, "Preview"),
        button("xlsx", "Excel"),
        button("csv", "CSV bundle"),
        button("json", "JSON"),
        button("pdf", "PDF")),
      status),
    preview));

  previewRegister();
}

// ---------------------------------------------------------------- routing ----

function route() {
  const hash = location.hash || "#/";
  stopPolling();
  exitDemoButton.hidden = !hash.startsWith("#/demo");
  demoLink.hidden = hash.startsWith("#/demo");
  if (hash === "#/demo") return demoHome();
  const journey = hash.match(/^#\/demo\/(invoice|document)\/([a-z0-9-]+)$/);
  if (journey) return demoJourney(journey[2], journey[1] === "invoice");
  const legacy = hash.match(/^#\/demo\/([a-z0-9-]+)$/);
  if (legacy) return demoJourney(legacy[1], true);
  if (hash === "#/trust") return trustPage();
  if (!key) return home("");
  setSignedIn(true);
  const register = hash.match(/^#\/register(?:\?ids=([0-9a-f,-]+))?$/i);
  if (register) return registerPage(register[1] || "");
  const match = hash.match(/^#\/d\/([0-9a-f-]{36})$/i);
  if (match) documentPage(match[1]);
  else if (hash === "#/settings") settingsPage();
  else if (hash === "#/about") aboutPage();
  else documents();
}

window.addEventListener("hashchange", route);
route();
