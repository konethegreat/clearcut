// Written by Kone Tshivhinda & Claude
import test from "node:test";
import assert from "node:assert/strict";
import { ClearCutClient, ClearCutError } from "./client.js";
import type { PageReading } from "./types.js";

const KEY = `ccut_${"a".repeat(32)}_${"b".repeat(64)}`;
const ANTHROPIC_KEY = "sk-ant-api03-not-a-real-key";
const DOC = "6f9619ff-8b86-4011-b42d-00c04fc964ff";

const READING: PageReading = {
  currency: "ZAR",
  amounts_include_vat: "no",
  invoice_number: "INV-1",
  invoice_date: "2026-10-01",
  supplier: { name: "Supplier", address: "", vat_number: "4123456789" },
  recipient: { name: "Customer", address: "1 Main Road", vat_number: "" },
  lines: [
    { description: "Widget", quantity: "2", unit_price: "100.00", line_total: "200.00", tax: "standard" },
  ],
  totals: { net: "200.00", vat: "30.00", gross: "230.00" },
  due: { date: "2026-10-31", terms: "30 days" },
  credit_note: { original_invoice_number: "", amount: "" },
};

/** A fetch that records every request and answers with `reply`. */
function recorder(reply: () => Response) {
  const calls: { url: string; init: RequestInit }[] = [];
  const impl = (async (url: URL | string | Request, init?: RequestInit) => {
    calls.push({ url: String(url), init: init ?? {} });
    return reply();
  }) as typeof fetch;
  return { calls, impl };
}

const reply = (body: unknown, status = 200) => () => new Response(JSON.stringify(body), { status });
const clientWith = (impl: typeof fetch) => new ClearCutClient({ baseUrl: "http://x", fetch: impl, apiKey: KEY });
const headerOf = (init: RequestInit, name: string) => new Headers(init.headers).get(name) ?? undefined;

// ---------- documents ----------

test("listDocuments asks for the tenant's documents and returns them", async () => {
  const documents = [
    {
      id: DOC,
      status: "needs_review",
      page_count: 2,
      original_filename: "march.pdf",
      created_at: "2026-10-27T10:00:00Z",
    },
  ];
  const { calls, impl } = recorder(reply({ documents }));
  assert.deepEqual(await clientWith(impl).listDocuments(), documents);
  const call = calls[0]!;
  assert.equal(call.url, "http://x/v1/documents");
  assert.equal(call.init.method ?? "GET", "GET");
  assert.equal(headerOf(call.init, "authorization"), `Bearer ${KEY}`);
});

test("getDocument asks for one document and returns its status and event count", async () => {
  const document = {
    id: DOC,
    status: "done",
    page_count: 1,
    original_filename: "march.pdf",
    events: 7,
  };
  const { calls, impl } = recorder(reply(document));
  assert.deepEqual(await clientWith(impl).getDocument(DOC), document);
  assert.equal(calls[0]!.url, `http://x/v1/documents/${DOC}`);
});

test("getEvents returns both the stored events and the table rows", async () => {
  const body = {
    events: [
      {
        id: "e1",
        date: "2026-10-01",
        payload: { type: "totals_stated", net: { minor: 20000, currency: "ZAR" }, vat: { minor: 3000, currency: "ZAR" }, gross: { minor: 23000, currency: "ZAR" } },
        citation: { document: DOC, page: 1 },
        extractor: { type: "model", name: "claude-haiku-5.5", prompt_version: "reading-v2" },
        confidence: 990,
      },
    ],
    table: [
      {
        id: "e1",
        date: "2026-10-01",
        group: "totals",
        type: "totals_stated",
        severity: "info",
        confidence_per_mille: 990,
        page: 1,
        text: "The document states a total of R230.00.",
      },
    ],
  };
  const { calls, impl } = recorder(reply(body));
  assert.deepEqual(await clientWith(impl).getEvents(DOC), body);
  assert.equal(calls[0]!.url, `http://x/v1/documents/${DOC}/events`);
});

test("getReport asks for a mode and returns the cited sentences", async () => {
  const report = {
    mode: "timeline",
    sections: [{ heading: null, sentences: [{ text: "A widget was billed.", cites: ["e1"] }] }],
  };
  const { calls, impl } = recorder(reply(report));
  assert.deepEqual(await clientWith(impl).getReport(DOC, { mode: "timeline" }), report);
  assert.equal(new URL(calls[0]!.url).searchParams.get("mode"), "timeline");
});

test("getReport uses the server's default when no mode is given", async () => {
  const { calls, impl } = recorder(reply({ mode: "summary", sections: [] }));
  await clientWith(impl).getReport(DOC);
  assert.equal(calls[0]!.url, `http://x/v1/documents/${DOC}/report`);
});

test("an unknown report mode is refused before any request", async () => {
  const { calls, impl } = recorder(reply({}));
  await assert.rejects(
    clientWith(impl).getReport(DOC, { mode: "wordy" as never }),
    TypeError,
  );
  assert.equal(calls.length, 0);
});

// ---------- export ----------

test("exportDocument returns the file's bytes, type and name", async () => {
  const bytes = new Uint8Array([0x25, 0x50, 0x44, 0x46]);
  const { calls, impl } = recorder(
    () =>
      new Response(bytes, {
        status: 200,
        headers: {
          "content-type": "application/pdf",
          "content-disposition": `attachment; filename="clearcut-${DOC}.pdf"`,
        },
      }),
  );
  const file = await clientWith(impl).exportDocument(DOC, { format: "pdf" });
  assert.deepEqual(file.bytes, bytes);
  assert.equal(file.contentType, "application/pdf");
  assert.equal(file.filename, `clearcut-${DOC}.pdf`);
  assert.equal(new URL(calls[0]!.url).searchParams.get("format"), "pdf");
});

test("exportDocument defaults to JSON and falls back to a safe file name", async () => {
  const { calls, impl } = recorder(
    () => new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
  );
  const file = await clientWith(impl).exportDocument(DOC);
  assert.equal(file.filename, `clearcut-${DOC}.json`);
  assert.equal(new URL(calls[0]!.url).searchParams.get("format"), "json");
});

test("a failed export is a ClearCutError, not a broken file", async () => {
  const { impl } = recorder(reply({ error: "document not found" }, 404));
  await assert.rejects(
    clientWith(impl).exportDocument(DOC, { format: "pdf" }),
    (e: unknown) => e instanceof ClearCutError && e.status === 404 && e.message === "document not found",
  );
});

test("an unknown export format is refused before any request", async () => {
  const { calls, impl } = recorder(reply({}));
  await assert.rejects(clientWith(impl).exportDocument(DOC, { format: "exe" as never }), TypeError);
  assert.equal(calls.length, 0);
});

// ---------- review ----------

test("getReviewQueue returns the pages waiting for a person", async () => {
  const pages = [{ page: 2, reason: "tax_not_shown", problems: [], findings: [], reading: READING }];
  const { calls, impl } = recorder(reply({ pages }));
  assert.deepEqual(await clientWith(impl).getReviewQueue(DOC), pages);
  assert.equal(calls[0]!.url, `http://x/v1/documents/${DOC}/review`);
});

test("confirmPage posts the person's reading and returns the queued job", async () => {
  const { calls, impl } = recorder(reply({ job_id: "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d" }));
  const result = await clientWith(impl).confirmPage(DOC, 2, READING);
  assert.deepEqual(result, { job_id: "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d" });
  const call = calls[0]!;
  assert.equal(call.url, `http://x/v1/documents/${DOC}/pages/2/review`);
  assert.equal(call.init.method, "POST");
  assert.deepEqual(JSON.parse(String(call.init.body)), READING);
});

test("a page number that is not a positive whole number is refused before any request", async () => {
  const { calls, impl } = recorder(reply({}));
  const client = clientWith(impl);
  for (const page of [0, -1, 1.5, Number.NaN, Number.POSITIVE_INFINITY, 4294967296]) {
    await assert.rejects(client.confirmPage(DOC, page, READING), TypeError, String(page));
  }
  assert.equal(calls.length, 0);
});

// ---------- delete ----------

test("deleteDocument sends DELETE and accepts the empty 204 answer", async () => {
  const { calls, impl } = recorder(() => new Response(null, { status: 204 }));
  assert.equal(await clientWith(impl).deleteDocument(DOC), undefined);
  const call = calls[0]!;
  assert.equal(call.url, `http://x/v1/documents/${DOC}`);
  assert.equal(call.init.method, "DELETE");
});

test("deleting a document that is being worked on is a 409 ClearCutError", async () => {
  const { impl } = recorder(
    reply({ error: "the document is being worked on; try again when it has finished" }, 409),
  );
  await assert.rejects(
    clientWith(impl).deleteDocument(DOC),
    (e: unknown) => e instanceof ClearCutError && e.status === 409,
  );
});

// ---------- the tenant's Anthropic key ----------

test("getAnthropicKey reports whether a key is set, never the key", async () => {
  const status = {
    configured: true,
    last4: "abcd",
    set_at: "2026-10-27T10:00:00Z",
    checked_at: "2026-10-27T10:00:00Z",
    check_ok: true,
    source: "tenant",
    mode: "tenant-only",
    tenant_keys_enabled: true,
  };
  const { calls, impl } = recorder(reply(status));
  const result = await clientWith(impl).getAnthropicKey();
  assert.deepEqual(result, status);
  assert.ok(!JSON.stringify(result).includes(ANTHROPIC_KEY));
  assert.equal(calls[0]!.url, "http://x/v1/anthropic-key");
});

test("setAnthropicKey puts the key in the JSON body and never in the URL", async () => {
  const stored = { last4: "abcd", models: ["claude-haiku-5.5"], checked_at: "2026-10-27T10:00:00Z" };
  const { calls, impl } = recorder(reply(stored));
  assert.deepEqual(await clientWith(impl).setAnthropicKey(ANTHROPIC_KEY), stored);
  const call = calls[0]!;
  assert.equal(call.url, "http://x/v1/anthropic-key");
  assert.equal(call.init.method, "PUT");
  assert.deepEqual(JSON.parse(String(call.init.body)), { key: ANTHROPIC_KEY });
  assert.ok(!call.url.includes(ANTHROPIC_KEY));
});

test("a refused Anthropic key is a ClearCutError that does not repeat the key", async () => {
  const { impl } = recorder(reply({ error: "Anthropic did not accept that key" }, 422));
  await assert.rejects(
    clientWith(impl).setAnthropicKey(ANTHROPIC_KEY),
    (e: unknown) => {
      assert.ok(e instanceof ClearCutError);
      assert.equal(e.status, 422);
      assert.ok(!e.message.includes(ANTHROPIC_KEY) && !String(e.stack).includes(ANTHROPIC_KEY));
      return true;
    },
  );
});

test("removeAnthropicKey deletes it and says whether there was one", async () => {
  const { calls, impl } = recorder(reply({ removed: true }));
  assert.deepEqual(await clientWith(impl).removeAnthropicKey(), { removed: true });
  assert.equal(calls[0]!.url, "http://x/v1/anthropic-key");
  assert.equal(calls[0]!.init.method, "DELETE");
});

// ---------- ids ----------

test("every document call refuses an id that is not a UUID before any request", async () => {
  const { calls, impl } = recorder(reply({}));
  const client = clientWith(impl);
  const bad = ["", " ", "42", "not-a-uuid", `${DOC}/../../x`, `${DOC}?x=1`, `${DOC}\n`];
  for (const id of bad) {
    await assert.rejects(client.getDocument(id), TypeError, JSON.stringify(id));
    await assert.rejects(client.getEvents(id), TypeError, JSON.stringify(id));
    await assert.rejects(client.getReport(id), TypeError, JSON.stringify(id));
    await assert.rejects(client.exportDocument(id), TypeError, JSON.stringify(id));
    await assert.rejects(client.getReviewQueue(id), TypeError, JSON.stringify(id));
    await assert.rejects(client.confirmPage(id, 1, READING), TypeError, JSON.stringify(id));
    await assert.rejects(client.deleteDocument(id), TypeError, JSON.stringify(id));
  }
  assert.equal(calls.length, 0);
});
