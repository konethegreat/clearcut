// Written by Kone Tshivhinda & Claude.
// Three screens, no framework. Everything that came off a document goes into the page with
// textContent (see h() below), never as markup. The ClearCut key lives in this variable only:
// it is not written to storage or a cookie, so reloading the page signs you out.
"use strict";

let key = null;
let timer = null;
const main = document.getElementById("main");
const signoutButton = document.getElementById("signout");

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

// ---------------------------------------------------------------- 1. sign in ----

function signIn(message) {
  stopPolling();
  key = null;
  signoutButton.hidden = true;
  const input = h("input", { type: "password", id: "key", autocomplete: "off", spellcheck: "false", required: true });
  const error = h("p", { class: "error small", role: "alert" }, message || "");
  const form = h(
    "form",
    {
      class: "card narrow",
      onsubmit: async (event) => {
        event.preventDefault();
        key = input.value.trim();
        input.value = "";
        try {
          const response = await api("/v1/whoami");
          if (!response.ok) throw new AuthError("The key was not accepted.");
          signoutButton.hidden = false;
          location.hash = "#/";
          route();
        } catch (e) {
          key = null;
          error.textContent = e.message || "Could not reach the server.";
        }
      },
    },
    h("h1", {}, "Sign in"),
    h("p", { class: "muted" }, "Paste your ClearCut key. It stays in this page only: reloading the page signs you out."),
    h("label", { for: "key" }, "ClearCut key"),
    input,
    error,
    h("button", { type: "submit" }, "Sign in"),
  );
  show(form);
  input.focus();
}

signoutButton.addEventListener("click", () => {
  location.hash = "#/";
  signIn("");
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
  show(h("h1", {}, "Documents"), h("p", { class: "muted" }, "Upload an invoice to have it read."), h("div", { class: "card" }, drop, status), h("h2", {}, "Your documents"), list);

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
  show(h("p", {}, h("a", { href: "#/" }, "← All documents")), body);
  let mode = "summary";

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
    const working = info.status === "received" || info.status === "processing";
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
          h("p", {}, job && job.progress_note ? "Working: " + job.progress_note : "Waiting to be read…"),
          p && p.total ? h("progress", { max: p.total, value: p.done }) : h("progress", {}),
          p && p.total ? h("p", { class: "small muted" }, "Step " + p.done + " of " + p.total) : null,
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
    if (info.events > 0) {
      parts.push(reportSection(), tableSection(events.table), downloads());
    } else if (!working) {
      parts.push(h("div", { class: "states" }, "Nothing was read from this document."));
    }
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

  function downloads() {
    const make = (format, label, type) =>
      h("button", {
        type: "button", class: "secondary",
        onclick: async (e) => {
          const button = e.currentTarget;
          button.disabled = true;
          try {
            const response = await api("/v1/documents/" + id + "/export?format=" + format);
            if (!response.ok) throw new Error("The download failed (" + response.status + ").");
            const url = URL.createObjectURL(new Blob([await response.text()], { type }));
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
    return h("div", {}, h("h2", {}, "Download"), h("div", { class: "row" }, make("json", "JSON", "application/json"), make("csv", "CSV (opens in Excel)", "text/csv"), make("md", "Markdown", "text/markdown")));
  }

  draw();
}

// ---------------------------------------------------------------- routing ----

function route() {
  if (!key) return signIn("");
  const hash = location.hash || "#/";
  const match = hash.match(/^#\/d\/([0-9a-f-]{36})$/i);
  if (match) documentPage(match[1]);
  else documents();
}

window.addEventListener("hashchange", route);
signIn("");
