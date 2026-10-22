# ADR-0009: The Python worker and the model-provider boundary

Status: **Accepted** (approved by Kone Tshivhinda, 2026-10-22, with all five leanings below) · Date: 2026-10-22 · Deciders: Kone Tshivhinda, Claude
Design for task P2-04. It refines [ADR-0004](0004-ingestion-and-narration-pipeline.md): the worker reads documents, and model calls live in Rust (see "The decision to confirm").

## Context
[ADR-0004](0004-ingestion-and-narration-pipeline.md) says a thin Python worker "owns OCR and model calls", holds no business rules, and sits behind a provider interface. [ADR-0005](0005-model-strategy-escalation.md) adds the escalation ladder, per-call logging, budgets and the rule that a document's text is never trusted. The job queue and runner (ADR-0008) now exist, so the pipeline's next layer is the code that actually reads a document and calls a model.

Two kinds of work were bundled into "the worker", and they have very different risk:

| | Reading a document | Calling a model |
|---|---|---|
| What it does | Rasterise pages, read the text layer, OCR scans | Send pages and rules to Claude, get structured JSON back |
| Input | **Untrusted files** parsed by C libraries (PDF, image and OCR engines) | Untrusted *text* inside a prompt we assemble |
| Needs a secret? | No | Yes: `ANTHROPIC_API_KEY` |
| Needs the internet? | No | Yes: one host |
| Best ecosystem | Python (PyMuPDF, ocrmypdf, Tesseract, docling) | Either; the API is plain HTTPS and JSON |

Parsing hostile files is where memory-safety bugs live (image and PDF parsers have a long record of them). A process that does that **and** holds the API key **and** can reach the internet is the worst place for a compromise: a malicious invoice that exploits the parser could read the key and send documents anywhere.

## The decision to confirm
**Split the two.** (Recommended; ADR-0004's wording is refined, not discarded.)

- **The Python worker reads documents and nothing else.** It gets file bytes, returns page counts, text, word positions and page images. It has **no API key, no network egress, no database and no object-store credentials**, runs as a non-root user on a private network, and is the only component that touches a hostile file.
- **Rust owns the model provider.** A `ModelProvider` trait lives in Rust, with an Anthropic adapter (HTTPS, `reqwest`) behind it, next to the things that already live in Rust: the escalation router, budgets, the context-pack builder, per-call logging, the tenant rules and the number guard. The API key stays in the Rust service's environment.

Why this is better than ADR-0004 as written:
1. **A compromise of the riskiest component leaks nothing.** No key to steal, nowhere to send data, no database to reach.
2. **One place enforces policy.** Budgets, prompt-cache layout, thinking and effort per tier, redaction before sending (P2-19), the rule that memory is written only from human-confirmed corrections, and the per-call audit all sit in one language beside the router that decides the tier. Splitting them across a network hop duplicates the contract for every call.
3. **No extra hop for every model call**, and no second place that must be kept in sync with the Rust escalation logic.
4. The worker becomes genuinely "thin and rule-free", which is what ADR-0001 promised.

What it costs:
- Rust has no official Anthropic SDK, so we write and maintain the request and response types for the slice of the API we use (messages with images, tools, `cache_control`, adaptive thinking, the batch endpoints). That slice is small and stable, and recorded-response tests guard it. New API features need a manual addition rather than an upgrade.
- ADR-0004's line "Python owns model calls" and the task wording in P2-13 ("adapter behind the provider trait") change: the adapter is Rust.

**If Kone prefers ADR-0004 as written** (model calls in Python), nothing else in this design changes: `ModelProvider` stays a Rust trait, and its implementation becomes an HTTP client to the worker instead of to Anthropic. The security case above is the only reason to prefer the split.

## Proposal in brief
1. **Two Rust traits, one per kind of work.** `DocumentProcessor` (inspect a file, extract a page's text, render a page) implemented by an HTTP client to the worker; `ModelProvider` (send a request, get structured output and usage) implemented by the Anthropic adapter. Both have fakes for tests, so the pipeline is developed and tested without Python or the network.
2. **HTTP with JSON** between Rust and the worker, not gRPC. Bytes go as the raw request body with the details in the query string; answers are JSON (or PNG for a rendered page). It is debuggable with `curl`, needs no code generator on Windows, and the worker's OpenAPI document is generated for free. Authenticated with a shared bearer secret (`CLEARCUT_WORKER_TOKEN`) on a private network.
3. **A walking skeleton first.** The worker starts with `GET /health` and `POST /v1/inspect` (page count and whether a text layer exists) so the whole path, Rust client to Python and back, is proven with real bytes before OCR work begins (P2-05 extends it).
4. **Errors map onto the queue's failure kinds** so retry behaviour is decided in one place (see below).

## The worker (Python, `services/worker`)
- **Stack:** Python 3.12 or newer; FastAPI + pydantic for typed requests and responses; uvicorn; `pypdf` (pure Python) for the skeleton's inspection, with PyMuPDF and OCR engines arriving in P2-05. Dependencies pinned exactly. `pytest`, `ruff` and `mypy` in CI.
- **Endpoints (v1):** `GET /health` (no auth; status, version, capabilities) and `POST /v1/inspect` (body: file bytes; returns `{media_type, pages, has_text_layer}`). P2-05 adds `POST /v1/pages/{n}/text` and `POST /v1/pages/{n}/render`.
- **Hardening built in from the start:** bearer-token check with a constant-time comparison; a hard cap on request size and on page count; per-request timeout; nothing is written outside a per-request temporary directory that is deleted afterwards; no file path from the caller is ever opened; logs carry sizes and timings, **never content**; the same JSON error envelope as the Rust API (`{"error": "..."}`); runs as non-root, with a read-only filesystem and no egress when deployed (P1-07).
- **No business rules:** it does not validate totals, VAT or dates; it does not decide what a document is beyond what the bytes say.

## The Rust side
- **`clearcut-docproc`** (new crate): the `DocumentProcessor` trait, the request and result types, `HttpWorker` (the client: timeouts, bearer token, response size limits, error mapping) and `FakeWorker`.
- **`clearcut-provider`** (new crate): the `ModelProvider` trait and its types, and `FakeProvider`. The Anthropic adapter is P2-13, a separate task.
- **`ModelRequest`** carries the tier or model name, effort, system text, content blocks (text, page images), an output JSON Schema, a token cap, the static-prefix cache hints, and a call context (tenant, job, document, page, tier) used only for logging. **`ModelResponse`** carries the structured output, usage (input, output, cache read and write tokens), the stop reason and the request id. Tool definitions for the validator and calculator, and the context-pack builder, arrive with P2-16b and are not in the first version.
- **`ProviderError`:** `Unavailable` (overload, rate limit, 5xx, network), `Timeout`, `Truncated` (hit the token cap), `Refused`, `InvalidOutput` (not valid against the schema), `Rejected` (our request was wrong: a bug), `Auth`. The router in P2-14 decides what each means for escalation.

## How failures reach the queue
| What went wrong | `FailureKind` | Why |
|---|---|---|
| Worker or provider unreachable, overloaded, rate-limited | `ProviderUnavailable` | An outage is not the document's fault; it does not use up attempts |
| Timeout, 5xx from the worker, truncated output | `Transient` | A retry may succeed |
| Document is corrupt, encrypted, or over the size or page cap | `NeedsReview` | No retry will help; a person must look |
| Output fails schema or validation at the top tier | `NeedsReview` | ADR-0005's terminal state |

## Keeping the two sides in step
Cross-language drift is the known risk (ADR-0001). The worker publishes its OpenAPI document. A CI job installs the worker, starts it, and runs a Rust test (`clearcut-docproc`, behind an `#[ignore]` marker and an environment variable) that sends real bytes through `HttpWorker` and checks the answers. A change on either side that breaks the other fails CI. The worker's own pytest suite covers its endpoints, limits and error shapes.

## What gets built, in order (each its own small PR, tests first)
1. **`clearcut-provider`:** the trait, types and `FakeProvider`, with tests.
2. **The worker skeleton:** `services/worker` (app, auth, limits, `/health`, `/v1/inspect`), pytest suite, a CI job for Python, a README with how to run it.
3. **`clearcut-docproc`:** the trait, `HttpWorker`, `FakeWorker`, error mapping onto `FailureKind`, and the live contract test in CI.
4. (Later, with P1-07) a container image and compose service for the worker with no egress and no secrets.

## Implementation notes (step 1, `clearcut-provider`)
- **A request is checked when it is built**, against the API's real limits: a model id of plain identifier characters (at most 128), something to read, no blank text, an object output schema, an output cap of 1 to 64 000 tokens, images with bytes and at most 5 MiB each and 20 per request, at most 4 cache breakpoints across the whole request, and at most 25 MiB of text and image bytes (text counted in bytes, not characters). A request that could never be sent fails in our code with a named reason; one the provider still refuses is `ProviderError::Rejected`, which means a bug on our side.
- **Defaults** follow ADR-0005: adaptive thinking and `medium` effort. Which model belongs to which tier is the router's decision (P2-14), not this crate's.
- **Nothing prints invoice content.** The `Debug` output of a request, a text or image block and a response shows sizes and ids only, so a stray `{:?}` in a log line cannot leak a document. Tests assert this for every type.
- **Failed calls that were billed carry their usage** (`Truncated`, `Refused`, `InvalidOutput`), so budgets (P2-12) count what a failure cost.
- **Errors map onto the queue's failure kinds** as the table above says (`Unavailable` and `Auth` to provider-unavailable, so a bad key never burns a document's attempts; `Timeout`, `Truncated` and `InvalidOutput` to transient; `Refused` and `Rejected` to needs-review), and `Unavailable` can carry the provider's `retry_after`.
- **`FakeProvider`** gives scripted answers in order (or answers from the request), records every request, and says so loudly when a test asks for more than it scripted. It is public so the pipeline crates can use it in their own tests.
- Tool definitions for the validator and calculator, and the context-pack builder, are not in the first version; they arrive with P2-16b.

## Implementation notes (step 2, the worker skeleton)
- **Layout:** `services/worker` with a `src/clearcut_worker` package (`app`, `config`, `inspect`, `logs`, `__main__`), a pytest suite, `pyproject.toml` (strict `mypy`, `ruff` with the security rules on, warnings as errors in tests), and exact pins: direct dependencies in `requirements*.txt`, everything they pull in in `constraints.txt`.
- **Authentication before the body.** `/health` is public; `/v1/inspect` needs `Authorization: Bearer <token>`. Every failure is the same `401` with `WWW-Authenticate: Bearer`, and the check runs before a byte of the body is read (a test sends a huge body without a key and gets a 401, not a 413). The comparison is constant-time. The token must be at least 32 visible characters or the worker **refuses to start**.
- **The file type comes from the bytes** (`%PDF-` at offset 0), never from the caller's `Content-Type`. Only PDFs in this skeleton; images and the rest arrive with P2-05, and anything else is a `415`.
- **Limits:** a request-size cap enforced as the bytes arrive (so a chunked body with no declared length is cut off too), a page cap, and a deadline checked between pages. An encrypted PDF is refused, never opened (no password is ever tried). A page whose text cannot be pulled out counts as having none, and only the *kind* of error is logged.
- **Every inspection runs in a child process the parent can kill** (added after an automated security review of the first draft). A deadline checked between pages cannot stop a parser stuck inside one call, and a crash or out-of-memory kill would take the worker with it. So `inspect_isolated` starts a fresh interpreter (`spawn`, never `fork` in a process with threads), waits for the timeout plus a short grace period, and **kills the child** if it has not answered. A child that crashes is reported as "could not be read" and the worker carries on. Tests show a stuck parser killed at the limit with no process left behind, a child that dies hard, and several inspections at once.
- **The child cannot reach the parent through its answer.** The answer is a few bytes of JSON, read with a hard size limit and checked field by field (real non-negative integers, text pages never more than pages, only known refusal names). It is **never unpickled**: unpickling is running code, so a compromised parser could otherwise have used the channel to escape the box. A test sends a malicious pickle from the child and asserts the parent runs nothing. Memory limits for the child belong to the container (P1-07).
- **Errors say which rule failed and never repeat the file.** Parser messages are swallowed; the answer is "the document could not be read". JSON everywhere (`{"error": "..."}`), including 404, 405, framework validation errors and unexpected failures.
- **Nothing in a log carries content or the token, and a caller cannot forge a log line.** The log line is method, path, status, size and time. A known route is logged by its fixed template; an unknown path is escaped (control characters and non-ASCII become `\n`, `\x1b`, `\u2028` and so on) and cut to 200 characters, and the method is escaped too, so one request is always exactly one line. (Python's URL parser happened to drop a decoded newline from `request.url.path`; the worker logs the path from the ASGI scope instead so nothing is dropped silently and the escape is visible.) A first smoke test also found the request lines never appeared at all (uvicorn only configures its own loggers), so the worker sets up its logging on start.
- **The API description is not served** (`/docs` and `/openapi.json` answer 404); it is generated in code for the contract test in step 3 (`create_app(settings).openapi()`).
- **`httpx2`, not `httpx`,** is the test client dependency: Starlette now deprecates the old one, and the test configuration turns warnings into errors.
- **CI:** a `worker` job runs `ruff`, `ruff format --check`, `mypy` and `pytest` on Python 3.12 and 3.14.

## Alternatives considered
- *Everything in Python, as ADR-0004 words it.* Rejected as the default for the security reason above; remains a fallback that changes only the provider's implementation.
- *Everything in Rust, no Python.* Rust can call PDFium and Tesseract, but layout-aware OCR and table extraction are far stronger in the Python ecosystem today, and Python is the one language already approved for this. Revisit if the worker's maintenance cost outweighs that.
- *gRPC.* Needs a protobuf toolchain on every developer's machine (a real cost on Windows) and code generation in two languages, for a service that is called a few times per page. HTTP and JSON are enough.
- *Running the worker as a library inside the Rust process (PyO3).* Puts the hostile-file parser in the same process as the database credentials and the API key. The opposite of the goal.
- *A message queue between Rust and the worker.* The job queue (ADR-0008) already makes the work asynchronous and retryable; the worker call is a plain request inside a job.

## Risks and notes
- **Untrusted documents.** Nothing the worker returns is trusted: its text is data for the model prompt and for validation, never instructions. The model's output is validated in Rust (`clearcut-core`) exactly as ADR-0004 requires.
- **Page images and POPIA.** Pages go to Anthropic (outside South Africa, POPIA s72). Redaction of bank and ID numbers before sending (P2-19) happens in Rust, in front of the provider, now in one place.
- **Resource limits.** Rasterising a large scan can use a lot of memory. The size and page caps, a per-request timeout and container memory limits are part of the skeleton, not an afterthought.
- **Cost in Rand.** This ADR adds no model spend. The worker runs on our own compute; the per-page model cost is as in ADR-0005 (assumed R18/USD).

## Decisions (Kone, 2026-10-22)
1. **The worker is split from the model provider.** The Python worker only reads documents; the `ModelProvider` trait and the Anthropic adapter live in Rust. ADR-0004 is refined accordingly, and P2-13 becomes a Rust adapter.
2. **HTTP and JSON** between Rust and the worker (not gRPC).
3. **Python 3.12 or newer with FastAPI, pydantic and pypdf**, exact-pinned, with `ruff` and `mypy` in CI.
4. **Deployment constraints** are requirements (no egress, no secrets, non-root, private network, size and page caps), built with P1-07.
5. **The walking skeleton** (health plus inspect, proven end to end before OCR) is the first slice.
