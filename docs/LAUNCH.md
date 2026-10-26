# Launch checklist

_Kone Tshivhinda & Claude, 26 Oct 2026. The one place that says what is done, what is being built, what only Kone can do, and what is deliberately later. If something is open and is not here, add it here. Detailed task ids are in [PHASES.md](PHASES.md); decisions in the ADRs._

**The goal this list serves:** ClearCut deployed so that **people use it with their own Anthropic key**, **TimeLex (and other products) can call it**, and **nobody's money or invoices are put at risk**. Not a standalone app: a service with a REST API, an MCP server, an SDK and a plain web page, over one Rust core.

## 1. Works today (built, tested, tried against the real API)
- Upload PDF, PNG, JPEG (streamed, size-limited, content-addressed, de-duplicated); page text, OCR (English, Afrikaans), page pictures; text-layer trust check (a lying text layer is detected and the page read as a picture).
- Reading every page with a model, cheapest first (Haiku, then Sonnet, Opus, Fable only when a deterministic check fails), budgets in rand (R5 a document, R100 a month), resumable, every call logged with its cost. **A missing tax goes to a person instead of up the ladder** (6 invoices: R5.36 before, R0.13 after).
- Deterministic checks in Rust (lines, VAT, totals, SA tax-invoice fields), events as the source of truth, narration with the number guard, report modes, JSON / CSV / Markdown export.
- **Review:** a person corrects and confirms a page the reader could not settle; recorded as theirs, no model call.
- **Delete:** a document, everything read from it and its file are erased for good (POPIA); a content-free record of the deletion stays.
- Tenants, API keys with scopes, row-level security, audit log; the web page (sign in, upload, list, report, review, delete); `docker compose up` on your own computer with the worker and database isolated.
- Your own key on your own computer (mode A): environment or file, never printed, never in the worker. **Each account's own key on a deployed service (mode B)**: stored encrypted, write-only, used per job.
- Prompt-injection handling: document text is data; flagged pages get a person's eyes before "done".

## 2. Being built, in this order
Each is a small pull request with its own tests. Items marked **[Kone]** need a decision first.

| # | What | Why it is on the list |
|---|---|---|
| 1 | ~~**Users' own keys on a deployed service (ADR-0011 mode B, ADR-0019)**~~ **Built**: each tenant's key stored encrypted (AES-256-GCM, tenant bound in), write-only (`PUT/GET/DELETE /v1/anthropic-key`, `keys:admin`), the job runner uses the tenant's key, `CLEARCUT_KEY_MODE=tenant-only` never uses the server's, a document with no key is analysed and waits, a Settings screen, `generate-master-key`. **Remaining:** the master key is not yet rotated automatically (re-enter keys to rotate) | Deployment: users input their own key; TimeLex sets it once through the API. |
| 2 | **TimeLex answers and contract**: ~~`docs/integration/timelex.md`~~ done, ~~OpenAPI file (P1-08)~~ done (`contracts/openapi.json`, generated, with a staleness test), ~~the invoice calculation with `line_total` as given and its cents test pair~~ done. **Remaining:** idempotency keys on mutating calls (P1-05), a `contracts/examples/` folder of golden request and response files, and a tenant-creation endpoint (waits for TimeLex to say what it needs) | Another product depends on a stable contract. |
| 3 | **Exports in the formats promised**: PDF, Excel, Word, HTML, plain text, all from stored events through one export schema (P4-01 to P4-07) | Part of the product's core promise; only JSON, CSV and Markdown exist. |
| 4 | **MCP server up to date and complete**: the 2026-07-28 spec, tools `upload_document`, `get_events`, `narrate_document`, `export_document`, `review_extraction` (P1-06, P2-11, P3-10, P4-09) | The agent front door; today it has 3 tools. |
| 5 | **Protecting users' money**: spend gate for documents at the same time and the month's ledger (P2-12), rate-limit-aware client (P2-24), calls of a page that dies mid-page are logged, retry delay on failures, a job waiting on a key problem waits instead of burning attempts | A user's own money must not be spent by a bug or a race. |
| 6 | **Hardening**: terminal-state guarantee with chaos tests (P2-18: every job ends `done` or `needs_review`), duplicate-invoice detection (P2-08), carried balances and statement totals across pages (P2-07) | Correctness on real documents. |
| 7 | **Deploy path**: CI builds the images, HTTPS (Caddy) in compose, S3-compatible storage, `docs/DEPLOY.md` (backups, upgrades, the key switch), the hosted-trial path (ADR-0015) when it is decided who runs it | Something others can run. |
| 8 | **Security review and threat model** (P8-05) including prompt injection from documents and the key store; **POPIA drafts**: privacy notice and operator agreement templates, retention and deletion description, for a lawyer to review | Before anyone else's invoices. |
| 9 | **Review and audit UX**: the page picture beside the form, reject or re-read a page, an audit-trail view, citation jump-to-source (P3-08, P5-02, P5-04), raw output and prompt versions stored for reproducibility (P3-09) | Makes review fast and results traceable. |

## 3. Only Kone can do these
- **Real invoices** (anonymised, with consent) for the gold set (P2-01), so accuracy is measured instead of claimed, and `prompt-v1` can be frozen. Until then nothing here claims "0 silent critical errors": it is a target.
- **Who runs ClearCut commercially**, hence the POPIA responsible party and Information Officer; an attorney's review of the privacy notice, consent text and operator agreement (P2-19).
- **Anthropic terms**: confirm retention and zero-data-retention for the models in use (research 06 section 8: Fable 5.1 is 30-day retention).
- **Hosting and a domain**, when a hosted trial or service is wanted.
- **Which documents beyond invoices** (bank, payment-processor, supplier statements) for fee reconciliation (P2-21), and whose taxonomy for line categories (P2-22).

## 4. Deliberately later (and what brings each back)
| Item | Comes back when |
|---|---|
| Issuing invoices, gapless numbering, credit notes, delivery (Phase 6) | Reading and exporting is live and a customer asks to issue |
| E-invoicing: UBL / Peppol, SARS rules (Phase 7) | SARS dates firm up, or TimeLex needs it |
| Payments, reconciliation, connectors (Phase 8) | A user asks for it after Phase 6 |
| Batch API mode (P2-16), memory store (P2-17), LLM prose smoothing (P3-05), line categorisation (P2-22) | The gold set shows where they would help |
| Embedded XML reader: UBL / CII / Factur-X (P2-03) | Real invoices with embedded XML turn up |
| Decoding glyph-obfuscated PDFs instead of reading the picture (P2-20 part 3) | Picture reading proves too costly or inaccurate |
| Resumable chunked uploads | Files near the size limit are common |
| A second person approving a review | A customer needs separation of duties |

## 5. Known limits today (so nothing is a surprise)
- Set `CLEARCUT_KEY_MODE=tenant-only` on any deployment: in the default `shared` mode the server's own key fills in for an account with none, which is right for your own computer and wrong for other people's money.
- One document is read at a time (deliberate until item 5).
- No HTTPS in the compose file; the page is bound to this computer only.
- If saving a review succeeds but queueing the finishing job fails, the document waits until a job is queued.
- A file whose deletion fails after its rows were erased is left behind (reported in the log) for a sweep that does not exist yet.
- Deleting a document is not locked against a job queued, or the same bytes uploaded again, in the same instant: the first makes that job fail on its foreign keys (nothing is written), the second can leave a row whose file is missing (reported as "file missing", fixed by uploading again). Both need a lock inside the database function; it goes into the next migration (item 1).
- Nothing is encrypted at rest in the database or file store: use disk encryption on the host.
