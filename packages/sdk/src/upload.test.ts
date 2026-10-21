// Written by Kone Tshivhinda & Claude
import test from "node:test";
import assert from "node:assert/strict";
import { ClearCutClient, ClearCutError } from "./client.js";

const KEY = `ccut_${"a".repeat(32)}_${"b".repeat(64)}`;
const PDF = new TextEncoder().encode("%PDF-1.7 a tiny invoice");
const STORED = {
  document_id: "d0c",
  duplicate: false,
  sha256: "ab".repeat(32),
  byte_size: PDF.length,
  filename: "march.pdf",
  media_type: "application/pdf",
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

const created = () => new Response(JSON.stringify(STORED), { status: 201 });
const clientWith = (impl: typeof fetch) => new ClearCutClient({ baseUrl: "http://x", fetch: impl, apiKey: KEY });
const headerOf = (init: RequestInit, name: string) => new Headers(init.headers).get(name) ?? undefined;

test("uploadDocument posts the raw bytes, not JSON, with the key and the content type", async () => {
  const { calls, impl } = recorder(created);
  await clientWith(impl).uploadDocument(PDF, { contentType: "application/pdf", filename: "march.pdf" });
  const call = calls[0]!;
  assert.equal(call.init.method, "POST");
  assert.equal(new URL(call.url).pathname, "/v1/documents");
  assert.equal(headerOf(call.init, "content-type"), "application/pdf");
  assert.equal(headerOf(call.init, "authorization"), `Bearer ${KEY}`);
  assert.equal(call.init.body, PDF, "the bytes must be sent as they are");
});

test("it returns what the server says, including for a repeat upload", async () => {
  const { impl } = recorder(created);
  assert.deepEqual(await clientWith(impl).uploadDocument(PDF, { contentType: "application/pdf" }), STORED);

  const again = { ...STORED, duplicate: true };
  const repeat = recorder(() => new Response(JSON.stringify(again), { status: 200 }));
  const result = await clientWith(repeat.impl).uploadDocument(PDF, { contentType: "application/pdf" });
  assert.equal(result.duplicate, true);
});

test("the file name travels as a query parameter and survives odd characters", async () => {
  const name = "Café & Co/March #1 100%.pdf";
  const { calls, impl } = recorder(created);
  await clientWith(impl).uploadDocument(PDF, { contentType: "application/pdf", filename: name });
  const url = new URL(calls[0]!.url);
  assert.equal(url.searchParams.get("filename"), name);
  assert.equal([...url.searchParams.keys()].length, 1);
});

test("without a file name there is no query string at all", async () => {
  const { calls, impl } = recorder(created);
  await clientWith(impl).uploadDocument(PDF, { contentType: "application/pdf" });
  assert.equal(calls[0]!.url, "http://x/v1/documents", "not even a bare ?");
});

test("a Blob supplies its own content type, which an explicit one overrides", async () => {
  const blob = new Blob([PDF], { type: "image/png" });
  const first = recorder(created);
  await clientWith(first.impl).uploadDocument(blob);
  assert.equal(headerOf(first.calls[0]!.init, "content-type"), "image/png");
  assert.equal(first.calls[0]!.init.body, blob);

  const second = recorder(created);
  await clientWith(second.impl).uploadDocument(blob, { contentType: "application/pdf" });
  assert.equal(headerOf(second.calls[0]!.init, "content-type"), "application/pdf");
});

test("an ArrayBuffer is accepted too", async () => {
  const buffer = PDF.buffer.slice(PDF.byteOffset, PDF.byteOffset + PDF.byteLength) as ArrayBuffer;
  const { calls, impl } = recorder(created);
  await clientWith(impl).uploadDocument(buffer, { contentType: "application/pdf" });
  assert.equal(calls[0]!.init.body, buffer);
});

test("with no content type from anywhere it refuses before any request is made", async () => {
  const { calls, impl } = recorder(created);
  await assert.rejects(clientWith(impl).uploadDocument(PDF), TypeError);
  await assert.rejects(clientWith(impl).uploadDocument(new Blob([PDF])), TypeError, "a Blob with no type has none");
  await assert.rejects(clientWith(impl).uploadDocument(PDF, { contentType: "" }), TypeError);
  assert.equal(calls.length, 0);
});

test("a content type that could inject a header is refused before any request", async () => {
  const { calls, impl } = recorder(created);
  for (const bad of ["application/pdf\r\nX-Evil: 1", "application/pdf\n", "application/pdf\u0000"]) {
    await assert.rejects(clientWith(impl).uploadDocument(PDF, { contentType: bad }), TypeError, JSON.stringify(bad));
  }
  assert.equal(calls.length, 0);
});

test("each refusal becomes a ClearCutError with the server's words", async () => {
  const cases: [number, object, string][] = [
    [413, { error: "file too large", max_bytes: 1000 }, "file too large"],
    [415, { error: "unsupported media type" }, "unsupported media type"],
    [400, { error: "the file is empty" }, "the file is empty"],
    [401, { error: "authentication failed" }, "authentication failed"],
  ];
  for (const [status, body, message] of cases) {
    const { impl } = recorder(() => new Response(JSON.stringify(body), { status }));
    await assert.rejects(clientWith(impl).uploadDocument(PDF, { contentType: "application/pdf" }), (e: unknown) => {
      assert.ok(e instanceof ClearCutError);
      assert.equal(e.status, status);
      assert.equal(e.message, message);
      assert.ok(!String(e.stack).includes(KEY));
      return true;
    });
  }
});

test("a 403 says the key needs documents:write", async () => {
  const reply = () => new Response(JSON.stringify({ error: "forbidden", required_scope: "documents:write" }), { status: 403 });
  await assert.rejects(
    clientWith(recorder(reply).impl).uploadDocument(PDF, { contentType: "application/pdf" }),
    (e: unknown) => e instanceof ClearCutError && e.status === 403 && e.requiredScope === "documents:write",
  );
});

test("JSON calls still send JSON after the change", async () => {
  const { calls, impl } = recorder(() => new Response("{}", { status: 200 }));
  await clientWith(impl).calculate({ number: "1", currency: "ZAR", status: "draft", lines: [] });
  assert.equal(headerOf(calls[0]!.init, "content-type"), "application/json");
});
