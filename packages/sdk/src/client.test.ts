// Written by Kone Tshivhinda & Claude
import test from "node:test";
import assert from "node:assert/strict";
import { ClearCutClient, ClearCutError } from "./client.js";

test("calculate posts invoice JSON", async () => {
  const fake = (async (_url: URL | string | Request, init?: RequestInit) => {
    assert.equal(init?.method, "POST");
    return new Response(JSON.stringify({ display_gross: "R1.15" }), { status: 200 });
  }) as typeof fetch;
  const c = new ClearCutClient("http://x", fake);
  const r = await c.calculate({ number: "1", currency: "ZAR", status: "draft", lines: [] });
  assert.equal(r.display_gross, "R1.15");
});

test("surfaces API errors", async () => {
  const fake = (async () => new Response(JSON.stringify({ error: "invoice has no line items" }), { status: 422 })) as typeof fetch;
  const c = new ClearCutClient("http://x", fake);
  await assert.rejects(c.calculate({ number: "1", currency: "ZAR", status: "draft", lines: [] }), (e: unknown) => e instanceof ClearCutError && e.status === 422);
});
