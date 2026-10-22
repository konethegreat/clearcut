// Written by Kone Tshivhinda & Claude
import test from "node:test";
import assert from "node:assert/strict";
import { ClearCutClient, ClearCutError } from "./client.js";
import type { Job } from "./types.js";

const KEY = `ccut_${"a".repeat(32)}_${"b".repeat(64)}`;
const DOC = "6f9619ff-8b86-4011-b42d-00c04fc964ff";
const JOB = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

const JOB_BODY: Job = {
  id: JOB,
  document_id: DOC,
  kind: "process_document",
  state: "running",
  attempts: 1,
  max_attempts: 5,
  progress: { done: 3, total: 12, percent: 25 },
  progress_note: "page 3 of 12",
  last_error: null,
  cancel_requested: false,
  created_at: "2023-11-14T22:13:20Z",
  updated_at: "2023-11-14T22:13:40Z",
  started_at: "2023-11-14T22:13:25Z",
  finished_at: null,
  run_after: "2023-11-14T22:13:30Z",
  deadline_at: "2023-11-16T22:13:20Z",
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

// ---------- listing ----------

test("jobsForDocument asks for the document's jobs with the key and returns the list", async () => {
  const { calls, impl } = recorder(reply({ jobs: [JOB_BODY] }));
  const jobs = await clientWith(impl).jobsForDocument(DOC);
  assert.deepEqual(jobs, [JOB_BODY]);
  const call = calls[0]!;
  assert.equal(call.url, `http://x/v1/documents/${DOC}/jobs`);
  assert.equal(call.init.method ?? "GET", "GET");
  assert.equal(headerOf(call.init, "authorization"), `Bearer ${KEY}`);
});

test("a document with no jobs gives an empty list", async () => {
  const { impl } = recorder(reply({ jobs: [] }));
  assert.deepEqual(await clientWith(impl).jobsForDocument(DOC), []);
});

test("an unknown document is a 404 ClearCutError", async () => {
  const { impl } = recorder(reply({ error: "document not found" }, 404));
  await assert.rejects(clientWith(impl).jobsForDocument(DOC), (e: unknown) => {
    assert.ok(e instanceof ClearCutError);
    assert.equal(e.status, 404);
    assert.equal(e.message, "document not found");
    assert.ok(!String(e.stack).includes(KEY));
    return true;
  });
});

test("listing without documents:read says which scope was missing", async () => {
  const { impl } = recorder(reply({ error: "forbidden", required_scope: "documents:read" }, 403));
  await assert.rejects(
    clientWith(impl).jobsForDocument(DOC),
    (e: unknown) => e instanceof ClearCutError && e.status === 403 && e.requiredScope === "documents:read",
  );
});

// ---------- cancelling ----------

test("cancelJob posts to the job's cancel address with the key", async () => {
  const { calls, impl } = recorder(reply({ job: { ...JOB_BODY, state: "cancelled" } }));
  await clientWith(impl).cancelJob(JOB);
  const call = calls[0]!;
  assert.equal(call.url, `http://x/v1/jobs/${JOB}/cancel`);
  assert.equal(call.init.method, "POST");
  assert.equal(headerOf(call.init, "authorization"), `Bearer ${KEY}`);
});

test("a job cancelled at once (200) is not 'stopping'", async () => {
  const cancelled = { ...JOB_BODY, state: "cancelled" as const, finished_at: "2023-11-14T22:14:00Z" };
  const { impl } = recorder(reply({ job: cancelled }, 200));
  assert.deepEqual(await clientWith(impl).cancelJob(JOB), { job: cancelled, stopping: false });
});

test("a job that was only asked to stop (202) is 'stopping'", async () => {
  const asked = { ...JOB_BODY, cancel_requested: true };
  const { impl } = recorder(reply({ job: asked }, 202));
  assert.deepEqual(await clientWith(impl).cancelJob(JOB), { job: asked, stopping: true });
});

test("a job that had already finished comes back as it was (200)", async () => {
  const done = { ...JOB_BODY, state: "succeeded" as const, finished_at: "2023-11-14T22:14:00Z" };
  const { impl } = recorder(reply({ job: done }, 200));
  const result = await clientWith(impl).cancelJob(JOB);
  assert.equal(result.job.state, "succeeded");
  assert.equal(result.stopping, false);
});

test("cancelling an unknown job is a 404 and without documents:write a 403 naming the scope", async () => {
  const missing = recorder(reply({ error: "job not found" }, 404));
  await assert.rejects(
    clientWith(missing.impl).cancelJob(JOB),
    (e: unknown) => e instanceof ClearCutError && e.status === 404 && e.message === "job not found",
  );
  const forbidden = recorder(reply({ error: "forbidden", required_scope: "documents:write" }, 403));
  await assert.rejects(
    clientWith(forbidden.impl).cancelJob(JOB),
    (e: unknown) => e instanceof ClearCutError && e.status === 403 && e.requiredScope === "documents:write",
  );
});

// ---------- ids ----------

test("an id that is not a UUID is refused before any request is made", async () => {
  const { calls, impl } = recorder(reply({ jobs: [] }));
  const client = clientWith(impl);
  const bad = ["", " ", "42", "not-a-uuid", `${DOC}0`, `0${DOC}`, DOC.replace(/-/g, ""), `${DOC}/../../x`, `${DOC}?x=1`, `../${DOC}`, `${DOC} `, `${DOC}\n`, "6f9619ff8b8640 11b42d00c04fc964ff"];
  for (const id of bad) {
    await assert.rejects(client.jobsForDocument(id), TypeError, JSON.stringify(id));
    await assert.rejects(client.cancelJob(id), TypeError, JSON.stringify(id));
  }
  assert.equal(calls.length, 0);
});

test("an upper-case UUID is accepted", async () => {
  const { calls, impl } = recorder(reply({ jobs: [] }));
  await clientWith(impl).jobsForDocument(DOC.toUpperCase());
  assert.equal(calls.length, 1);
});

test("the error for a bad id never repeats the key", async () => {
  await assert.rejects(
    clientWith(recorder(reply({})).impl).cancelJob("nope"),
    (e: unknown) => e instanceof TypeError && !e.message.includes("ccut_"),
  );
});
