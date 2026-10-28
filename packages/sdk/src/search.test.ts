// Written by Kone Tshivhinda & Claude
import test from "node:test";
import assert from "node:assert/strict";
import { ClearCutClient } from "./client.js";

const KEY = `ccut_${"a".repeat(32)}_${"b".repeat(64)}`;
const DOC = "6f9619ff-8b86-4011-b42d-00c04fc964ff";

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

const RESULTS = {
  query: "Ndlovu",
  matches: [
    {
      document_id: DOC,
      status: "done",
      page_count: 1,
      original_filename: "march.pdf",
      created_at: "2026-10-27T10:00:00Z",
      event_id: `${DOC}:r1:supplier`,
      type: "supplier_identified",
      page: 1,
      excerpt: "Ndlovu Consulting (Pty) Ltd",
    },
  ],
};

test("searchDocuments sends the query and returns the matching events", async () => {
  const { calls, impl } = recorder(reply(RESULTS));
  assert.deepEqual(await clientWith(impl).searchDocuments("Ndlovu"), RESULTS);
  const call = calls[0]!;
  assert.equal(call.url, "http://x/v1/search?q=Ndlovu");
  assert.equal(call.init.method ?? "GET", "GET");
  assert.equal(new Headers(call.init.headers).get("authorization"), `Bearer ${KEY}`);
});

test("searchDocuments trims the query and sends a limit when one is given", async () => {
  const { calls, impl } = recorder(reply({ query: "Acme", matches: [] }));
  assert.deepEqual(await clientWith(impl).searchDocuments("  Acme  ", { limit: 5 }), {
    query: "Acme",
    matches: [],
  });
  assert.equal(calls[0]!.url, "http://x/v1/search?q=Acme&limit=5");
});

test("searchDocuments refuses a query the server would refuse, without sending it", async () => {
  const { calls, impl } = recorder(reply(RESULTS));
  const client = clientWith(impl);
  await assert.rejects(client.searchDocuments(" a "), TypeError);
  await assert.rejects(client.searchDocuments("x".repeat(101)), TypeError);
  assert.equal(calls.length, 0);
});

test("searchDocuments refuses a limit outside 1 to 200, without sending it", async () => {
  const { calls, impl } = recorder(reply(RESULTS));
  const client = clientWith(impl);
  await assert.rejects(client.searchDocuments("Acme", { limit: 0 }), TypeError);
  await assert.rejects(client.searchDocuments("Acme", { limit: 201 }), TypeError);
  await assert.rejects(client.searchDocuments("Acme", { limit: 2.5 }), TypeError);
  assert.equal(calls.length, 0);
});
