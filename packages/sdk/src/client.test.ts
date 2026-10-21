// Written by Kone Tshivhinda & Claude
import test from "node:test";
import assert from "node:assert/strict";
import { inspect } from "node:util";
import { ClearCutClient, ClearCutError } from "./client.js";

const KEY = `ccut_${"a".repeat(32)}_${"b".repeat(64)}`;
const EMPTY_INVOICE = { number: "1", currency: "ZAR", status: "draft", lines: [] } as const;

/** A fetch that records every request and answers with `reply`. */
function recorder(reply: () => Response) {
  const calls: { url: string; init: RequestInit }[] = [];
  const impl = (async (url: URL | string | Request, init?: RequestInit) => {
    calls.push({ url: String(url), init: init ?? {} });
    return reply();
  }) as typeof fetch;
  return { calls, impl };
}

function headerOf(init: RequestInit, name: string): string | undefined {
  const headers = new Headers(init.headers);
  return headers.get(name) ?? undefined;
}

test("calculate posts invoice JSON", async () => {
  const { calls, impl } = recorder(() => new Response(JSON.stringify({ display_gross: "R1.15" }), { status: 200 }));
  const c = new ClearCutClient({ baseUrl: "http://x", fetch: impl });
  const r = await c.calculate(EMPTY_INVOICE as never);
  assert.equal(r.display_gross, "R1.15");
  assert.equal(calls[0]?.init.method, "POST");
  assert.equal(calls[0]?.url, "http://x/v1/invoices/calculate");
});

test("surfaces API errors", async () => {
  const { impl } = recorder(() => new Response(JSON.stringify({ error: "invoice has no line items" }), { status: 422 }));
  const c = new ClearCutClient({ baseUrl: "http://x", fetch: impl, apiKey: KEY });
  await assert.rejects(c.calculate(EMPTY_INVOICE as never), (e: unknown) => e instanceof ClearCutError && e.status === 422);
});

test("sends the API key as a Bearer token on authenticated calls", async () => {
  const { calls, impl } = recorder(() => new Response("{}", { status: 200 }));
  const c = new ClearCutClient({ baseUrl: "http://x", fetch: impl, apiKey: KEY });
  await c.calculate(EMPTY_INVOICE as never);
  await c.whoami();
  assert.equal(calls.length, 2);
  for (const call of calls) assert.equal(headerOf(call.init, "authorization"), `Bearer ${KEY}`);
});

test("does not send the key to the public health endpoint", async () => {
  const { calls, impl } = recorder(() => new Response(JSON.stringify({ status: "ok", version: "0" }), { status: 200 }));
  const c = new ClearCutClient({ baseUrl: "http://x", fetch: impl, apiKey: KEY });
  await c.health();
  assert.equal(headerOf(calls[0]!.init, "authorization"), undefined);
});

test("sends no Authorization header when there is no key", async () => {
  const { calls, impl } = recorder(() => new Response("{}", { status: 200 }));
  const c = new ClearCutClient({ baseUrl: "http://x", fetch: impl });
  await c.whoami();
  assert.equal(headerOf(calls[0]!.init, "authorization"), undefined);
});

test("whoami returns the tenant, key id and scopes", async () => {
  const body = { tenant: "t", key_id: "k", scopes: ["documents:read", "keys:admin"] };
  const { impl } = recorder(() => new Response(JSON.stringify(body), { status: 200 }));
  const c = new ClearCutClient({ baseUrl: "http://x", fetch: impl, apiKey: KEY });
  assert.deepEqual(await c.whoami(), body);
});

test("a 401 becomes a ClearCutError that does not contain the key", async () => {
  const { impl } = recorder(() => new Response(JSON.stringify({ error: "authentication failed" }), { status: 401 }));
  const c = new ClearCutClient({ baseUrl: "http://x", fetch: impl, apiKey: KEY });
  await assert.rejects(c.whoami(), (e: unknown) => {
    assert.ok(e instanceof ClearCutError);
    assert.equal(e.status, 401);
    assert.equal(e.message, "authentication failed");
    assert.ok(!JSON.stringify(e).includes(KEY) && !String(e.stack).includes(KEY));
    return true;
  });
});

test("a 403 says which scope was missing", async () => {
  const reply = () => new Response(JSON.stringify({ error: "forbidden", required_scope: "documents:write" }), { status: 403 });
  const c = new ClearCutClient({ baseUrl: "http://x", fetch: recorder(reply).impl, apiKey: KEY });
  await assert.rejects(c.whoami(), (e: unknown) => {
    assert.ok(e instanceof ClearCutError);
    assert.equal(e.status, 403);
    assert.equal(e.requiredScope, "documents:write");
    return true;
  });
});

test("an error without a required scope has none", async () => {
  const { impl } = recorder(() => new Response(JSON.stringify({ error: "authentication failed" }), { status: 401 }));
  const c = new ClearCutClient({ baseUrl: "http://x", fetch: impl });
  await assert.rejects(c.whoami(), (e: unknown) => e instanceof ClearCutError && e.requiredScope === undefined);
});

test("the key does not show up if the client is logged or serialised", () => {
  const c = new ClearCutClient({ baseUrl: "http://x", apiKey: KEY });
  assert.ok(!JSON.stringify(c).includes(KEY));
  assert.ok(!inspect(c, { showHidden: true, depth: 5 }).includes(KEY));
  assert.ok(!String(c).includes(KEY));
  assert.ok(!Object.keys(c).some((k) => String((c as never)[k]).includes(KEY)));
});

test("a key that could inject a header is refused up front", () => {
  for (const bad of ["", "   ", `${KEY}\r\nX-Evil: 1`, `${KEY}\n`, `${KEY} `, ` ${KEY}`, "two words", "tab\there", "bell\u0007"]) {
    assert.throws(() => new ClearCutClient({ baseUrl: "http://x", apiKey: bad }), TypeError, JSON.stringify(bad));
  }
  assert.doesNotThrow(() => new ClearCutClient({ baseUrl: "http://x", apiKey: KEY }));
});

test("the error for a refused key never repeats the key", () => {
  const bad = `${KEY}\n`;
  assert.throws(
    () => new ClearCutClient({ apiKey: bad }),
    (e: unknown) => e instanceof TypeError && !e.message.includes("ccut_"),
  );
});

test("the default address is the local server", async () => {
  const { calls, impl } = recorder(() => new Response(JSON.stringify({ status: "ok", version: "0" }), { status: 200 }));
  await new ClearCutClient({ fetch: impl }).health();
  assert.equal(calls[0]?.url, "http://127.0.0.1:8080/health");
});
