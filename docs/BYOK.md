# Bring your own Anthropic key

_Kone Tshivhinda & Claude, 22 Oct 2026. Design: [ADR-0011](adr/0011-bring-your-own-anthropic-key.md). Facts: [research 06](research/06-anthropic-api.md)._

ClearCut has **no API key of its own and never will**. Whoever runs it uses their own Anthropic
API key, and Anthropic bills them directly. This page explains how to set that up safely.

**Status:** the key handling, the free key check and the adapter that talks to Anthropic with your key
(sections 2 to 5) exist and are tested against a stand-in server. The adapter has **not yet been run
against the real API**: section 4 has the one small, optional real call that does that, with your key.
The pages of a document are read by the model router and the whole path is tested against stand-ins; section 4 has the small real checks, the last of which reads one invented invoice page end to end. Nothing has been run against the real API yet.

## 1. What it costs

A planning estimate (not a promise; the first live runs will replace it with measured numbers),
at R18 per US dollar:

| Model | A 10-page invoice | Note |
|---|---|---|
| Haiku 5.5 (the default) | about R0.20 | cheapest; handles most clean invoices |
| Sonnet 5.5 | about R4 | used when the cheap model is not sure |
| Opus 5.5 | about R8 | harder pages |
| Fable 5.1 | about R20 | **off by default** (see section 6) |

The Batch API takes 50% off for bulk uploads. Because most invoices should stop at the cheapest model,
a 200-invoice test run costs in the order of tens of rand. ClearCut will also let you set a **budget**
(default R5 per document and R100 per month) and will stop, rather than overspend.

## 2. Make a safe key

In the [Claude Console](https://platform.claude.com):

1. Put ClearCut in **its own workspace** (Workspaces), so its usage, limits and files are separate
   from anything else you do.
2. Under that workspace, set a **monthly spend limit** you are comfortable losing. This is the
   strongest protection: it stops everything at that figure, whatever any program does.
3. Create an **API key** (API keys), name it `clearcut`, scope it to that workspace, and choose an
   **expiry** (30 days is a good habit; a leaked key then dies by itself).
4. Copy it once. It looks like `sk-ant-api03-...`.

ClearCut accepts only a regular API key. It refuses an **admin key** (which can manage your whole
organisation) and a **subscription login token** (not meant for other programs), with a clear message.

If a key leaks: **disable** it in the Console at once (disable can be undone; delete cannot), then
make a new one.

## 3. Give the key to ClearCut

Use one of these, never both:

```text
ANTHROPIC_API_KEY=sk-ant-api03-...            the key itself, in the environment
ANTHROPIC_API_KEY_FILE=/run/secrets/anthropic  a file that holds only the key
```

- In a container, mount the key as a file (Docker or Kubernetes secrets) and point
  `ANTHROPIC_API_KEY_FILE` at it. A mounted secret that is a link is followed.
- On Windows PowerShell, `$env:ANTHROPIC_API_KEY = "..."` lasts for that window only.
- **Never put the key on the command line.** Other users of the computer can read a program's
  arguments, so ClearCut refuses to start if it sees a key (or a flag like `--api-key`) there.
- Never commit the key. `.env` is already ignored by git, and CI scans for secrets, but the
  safest key is one that was never written into a project folder.

Two optional settings: `ANTHROPIC_WORKSPACE_ID` (only if your key is not tied to a single workspace)
and `CLEARCUT_ANTHROPIC_BASE_URL` (only for a local gateway; it must be `https`, or `http` to this
computer; it is the operator's setting and never something a user of a hosted ClearCut can choose).

## 3b. Everyone brings their own key: a deployed service (ADR-0019)

On your own computer one key in `.env` is enough (section 3). **When ClearCut is deployed for other
people, each account brings its own key**, so nobody's documents are read on someone else's money.

**For the person running the service**
1. Make a master key once: `docker compose run --rm admin generate-master-key` (or
   `cargo run -p clearcut-admin -- generate-master-key`). It prints one line. Put it in `.env` as
   `CLEARCUT_MASTER_KEY=<that line>` (or point `CLEARCUT_MASTER_KEY_FILE` at a file holding it) and **keep a copy
   somewhere safe**: without it every stored key is unreadable and has to be entered again. Keep it apart from the
   database and its backups: the database alone, or the master key alone, reveals nothing.
2. Set `CLEARCUT_KEY_MODE=tenant-only` in `.env`. Then the server's own `ANTHROPIC_API_KEY` is **never** used: an
   account with no key of its own has its documents analysed and then wait. (`shared`, the default, lets the server's
   key fill in for an account that has none: right for your own computer, wrong for other people's money.) The server
   refuses to start in `tenant-only` mode without a master key.

**For each user (or for TimeLex, once per tenant)**
- In the web page: **Settings**, paste the key, **Save key**. ClearCut asks Anthropic what the key can do (a free call,
  no tokens), refuses one Anthropic rejects, an admin key or a login token, and otherwise stores it **encrypted**
  (AES-256-GCM, tied to your account so it cannot be moved to another). It is **never shown again**: only its last four
  characters and whether Anthropic accepts it. **Remove my key** deletes it.
- Through the API (needs a ClearCut key with `keys:admin`): `PUT /v1/anthropic-key` with `{ "key": "sk-ant-..." }`,
  `GET /v1/anthropic-key` (status), `DELETE /v1/anthropic-key`. Nothing ever answers with the key.
- A document uploaded before a key is set is analysed and then **waits** (no attempts used, no cost) and starts within a
  minute of the key being saved. If Anthropic later refuses the key, the key is marked refused, documents wait, and the
  page asks for a new one. A refused key is never quietly replaced by the server's.
- Your own spend limit in the Anthropic Console is the one that cannot be bypassed; ClearCut's R5 a document and R100
  a month are a second line.

**What this protects, and what it does not.** A stolen database or backup holds only ciphertext. A stolen master key
without the database is useless. A stolen copy of **both** reads every key, so keep them apart. A compromised service
process holds the master key and can use any account's key, as the job runner does; that is why the advice is a
dedicated key with a spend limit and an expiry.

## 4. Check the key (free)

```text
ANTHROPIC_API_KEY=sk-ant-api03-... cargo test -p clearcut-anthropic --test live -- --ignored --nocapture
```

This asks Anthropic which models the key can use (`GET /v1/models`), which **costs nothing**, and prints
that, never the key (only its last four characters). If it fails, the message says who has to act:
the key is wrong or expired, a billing problem, the monthly cap was reached, or just a rate limit
(wait and retry).

To try one tiny real call through the adapter (**this one costs a fraction of a cent**, on the cheapest
model, and prints the tokens used and a rough bill in rand):

```text
ANTHROPIC_API_KEY=sk-ant-api03-... cargo test -p clearcut-anthropic --test live -- --ignored --nocapture one_tiny
```

To read one invented invoice page **end to end** (the real worker, the real model, a real database; about
R0.02, and the test caps the document at R1 and the month at R20). It needs the worker running (see its README,
with OCR if you can) and a database:

```text
ANTHROPIC_API_KEY=sk-ant-api03-... CLEARCUT_LIVE_WORKER_URL=http://127.0.0.1:8090 CLEARCUT_LIVE_WORKER_TOKEN=... DATABASE_URL=postgres://clearcut@127.0.0.1:5432/clearcut cargo test -p clearcut-pipeline --test live_read -- --ignored --nocapture
```

It prints which model read the page, every call with its tokens and what it cost in rand, and the total. Please
look at that output the first time: it is the first real measurement of the cost and the pass rate.

## 5. What ClearCut sends, and where

- Only to `api.anthropic.com`, over HTTPS. The key is in one request header and nowhere else. It never
  follows a redirect and never uses a proxy from the environment, so the key cannot be handed on.
- Page images and extracted text go with each request and are not kept at Anthropic by default.
  ClearCut will **not use Anthropic's Files API**, which would leave documents stored and visible to
  everything in the workspace.
- The Python document worker never sees the key.
- Every response's usage (tokens in and out, cache reads and writes) is recorded, so cost is a sum of
  facts, shown in rand.
- Nothing about who or what a call is for (tenant, job, document) is sent, and no tools, sampling
  settings or metadata. Anthropic's error messages are never copied into ClearCut's errors or logs,
  because they can quote what was sent.
- A rate limit that clears within a minute is waited out; an outage is retried twice; anything else
  (a bad key, a billing problem, the monthly cap) is reported at once and says who has to act.

## 6. Privacy and limits worth knowing

- By default Anthropic's API does not retain prompts and outputs, **except for Covered Models** such as
  Fable 5.1, which require 30-day retention and are refused to organisations on zero data retention.
  ClearCut keeps Fable off unless you turn it on.
- The Batch API (cheaper bulk) **does store data**; it is an opt-in.
- Your key shares its organisation's **rate limits** and **spend caps** with everything else you do.
  A new organisation may start in a low "Evaluation" tier. If you reach the monthly cap, the API stops
  until 00:00 UTC on the 1st; ClearCut will say so and wait, not retry in a loop.
- If you **host ClearCut for other people**, their invoices pass through you. You then carry the
  privacy responsibilities of POPIA, and need a privacy notice and an Information Officer. If you
  run it for your own invoices, those duties are yours already.
