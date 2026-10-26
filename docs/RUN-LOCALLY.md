# Running ClearCut on your own computer

_Kone Tshivhinda & Claude, 27 Oct 2026. This is the manual way; a one-command `docker compose` is task P1-07._

You need four things running: **Postgres** (the database), the **document worker** (reads PDFs; Python),
the **ClearCut server** (the API and the job runner), and your **Anthropic key** in a file.

## The easy way: `docker compose` (needs Docker Desktop)
1. In the project folder, make a file called `.env` with these lines (your Anthropic key is optional: without it documents are only analysed). The three made-up values must be **letters and digits only**, long, and different from each other:
   ```text
   ANTHROPIC_API_KEY=sk-ant-...your key...
   CLEARCUT_WORKER_TOKEN=any-long-random-letters-and-digits
   DB_OWNER_PASSWORD=another-long-random-letters-and-digits
   DB_APP_PASSWORD=a-third-long-random-letters-and-digits
   ```
2. Start everything (the first time it builds, which takes several minutes): `docker compose up --build -d`
3. Make your first login, once:
   ```text
   docker compose run --rm admin create-tenant "My firm"
   docker compose run --rm admin create-key --tenant <the id it printed> --name me --scopes documents:write,documents:read,keys:admin
   ```
   The second command prints a key **once**: keep it somewhere safe. It is your ClearCut login, not the Anthropic key.
4. Open **http://127.0.0.1:8099**, paste that key, and upload an invoice.

Stop with `docker compose down` (your documents stay); `docker compose down -v` also deletes them. The database and the document worker are on **separate** private networks with no internet, so the worker (which reads untrusted files) has no route to the database; the database needs passwords; only the server reaches Anthropic, and only this computer can reach the page.

## The manual way
(What compose does, by hand; useful when working on the code.)

## 1. Settings: one file, `.env`
Create a file called `.env` in the project folder (it is git-ignored, so it is never committed):

```text
ANTHROPIC_API_KEY=sk-ant-...your key...
DATABASE_URL=postgres://clearcut_app@127.0.0.1:54329/clearcut
CLEARCUT_STORAGE_DIR=C:\Users\You\clearcut-files
CLEARCUT_WORKER_URL=http://127.0.0.1:8090
CLEARCUT_WORKER_TOKEN=any-long-random-text-you-make-up
CLEARCUT_BIND=127.0.0.1:8099
```

- A real environment variable always wins over the file. The key is never printed or logged (only its last four characters).
- Without `ANTHROPIC_API_KEY` documents are analysed but no model reads them. Without the worker settings documents are stored but not read.
- Use a dedicated Anthropic key with a monthly spend limit (see `docs/BYOK.md`). A personal key (`sk-ant-usr-...`) works.

## 2. Start the parts
1. **Postgres:** `docker start clearcut-pg` (first time: the command in `CONTRIBUTING.md`).
2. **First time only, make the tables and a login for yourself** (this needs the *owner* connection, not the service one):
   ```text
   set DATABASE_URL=postgres://clearcut@127.0.0.1:54329/clearcut
   cargo run -p clearcut-admin -- migrate
   cargo run -p clearcut-admin -- create-tenant "My firm"            (prints the tenant id)
   cargo run -p clearcut-admin -- create-key --tenant <that id> --name me --scopes documents:write,documents:read,keys:admin
   ```
   The key is printed **once**: keep it. It is the password for the API, not the Anthropic key.
3. **The worker** (Python, from `services/worker`; see its README for installing): `python -m clearcut_worker` with
   `CLEARCUT_WORKER_TOKEN` set to the same text as in `.env`, `CLEARCUT_WORKER_PORT=8090` and `PYTHONPATH=src`.
4. **The server:** `cargo run -p clearcut-server`. It reads `.env`, starts the job runner next to the API, and says
   which key it uses (last four characters).

## 3. Use it
**The easy way:** open `http://127.0.0.1:8099/` in your browser (the address you set in `CLEARCUT_BIND`), paste your ClearCut key, choose an invoice, and watch it get read. The page shows the status, the report (summary, timeline, by group), everything that was read with the page and how sure it is, and download buttons for JSON, CSV and Markdown.

**With a command line:** upload a PDF (or PNG/JPEG). Replace `<KEY>` with the ClearCut key from step 2:

```text
curl -X POST "http://127.0.0.1:8099/v1/documents?filename=invoice.pdf" -H "Authorization: Bearer <KEY>" -H "Content-Type: application/pdf" --data-binary @invoice.pdf
```
It answers with a `document_id` and a `job_id` straight away; the reading happens in the background (about 10 seconds a page). Then:

| Ask | Address |
|---|---|
| Is it done? | `GET /v1/documents/{id}` (status `received`, `processing`, `done` or `needs_review`) |
| What is happening? | `GET /v1/documents/{id}/jobs` |
| The facts | `GET /v1/documents/{id}/events` |
| The story | `GET /v1/documents/{id}/report?mode=summary` (or `timeline`, `grouped`) |
| A file | `GET /v1/documents/{id}/export?format=json` (or `csv`, `md`) |
| Delete it (POPIA) | `DELETE /v1/documents/{id}` (needs `documents:write`; `409` while a job runs on it) |

Every sentence in a report cites the events it comes from. A document ends **`done`** when every page was read and the
whole document checks out (lines add up to the printed totals, the pages agree, nothing on a page tries to give
orders); otherwise it ends **`needs_review`** with the reason in the job's `last_error`, and the events are still there.

## Limits to know about (an honest list)
- One document at a time, on purpose: the shared monthly budget gate is not built (P2-12). The budget (R5 a document, R100 a month) is still enforced.
- The web page is plain on purpose: sign in, upload, a list of documents, one document's report, and **a review form** for a page the reader could not settle (for example an invoice that prints no tax: you choose the tax category from the original and confirm; ClearCut still checks the arithmetic, records the page as confirmed by your key, and finishes the document without asking a model). No accounts: the key is pasted each visit. Review needs a key with `documents:write` to confirm and `documents:read` to look.
- Nothing is encrypted at rest; run it on your own computer, not on a shared one.
- The first measurements: one invented page costs about R0.02 on Haiku. Real invoices will tell us more.
