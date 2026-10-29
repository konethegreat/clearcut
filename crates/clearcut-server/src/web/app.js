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
const aboutLink = document.getElementById("about-link");
const footerLinks = document.getElementById("footer-links");

function setSignedIn(signedIn) {
  signoutButton.hidden = !signedIn;
  settingsLink.hidden = !signedIn;
  aboutLink.hidden = !signedIn;
}

footerLinks.append(
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

// ---------------------------------------------------------------- 2. documents ----

async function documents() {
  stopPolling();
  const status = h("p", { class: "small muted", role: "status" }, "");
  const fileInput = h("input", { type: "file", accept: ".pdf,.png,.jpg,.jpeg,application/pdf,image/png,image/jpeg" });
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
  show(h("div", { class: "page" },
    h("div", { class: "page-head" },
      h("p", { class: "eyebrow" }, "Workspace"),
      h("h1", {}, "Documents"),
      h("p", { class: "lede" }, "Upload an invoice to have it read. Everything shown here is checked against the page it came from.")),
    h("div", { class: "card" }, drop, status),
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
              h("thead", {}, h("tr", {}, h("th", {}, "File"), h("th", {}, "Status"), h("th", { class: "num" }, "Pages"), h("th", {}, "Uploaded"))),
              h(
                "tbody",
                {},
                rows.map((d) =>
                  h(
                    "tr",
                    { class: "click", tabindex: "0", onclick: () => (location.hash = "#/d/" + d.id), onkeydown: (e) => { if (e.key === "Enter") location.hash = "#/d/" + d.id; } },
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
  amounts_include_vat: "The amounts on this page include VAT, which ClearCut does not back out yet. Enter the amounts without VAT, and answer \"No\" to \"Prices include VAT\".",
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
  amounts_include_vat: "includes VAT",
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

// ---------------------------------------------------------------- routing ----

function route() {
  if (!key) return home("");
  setSignedIn(true);
  const hash = location.hash || "#/";
  const match = hash.match(/^#\/d\/([0-9a-f-]{36})$/i);
  if (match) documentPage(match[1]);
  else if (hash === "#/settings") settingsPage();
  else if (hash === "#/about") aboutPage();
  else documents();
}

window.addEventListener("hashchange", route);
route();
