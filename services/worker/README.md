# ClearCut document worker

The Python half of document processing ([ADR-0009](../../docs/adr/0009-worker-and-provider-boundary.md)). It
**reads untrusted documents** and tells the Rust service what is in them. Nothing else.

| It does | It never does |
|---|---|
| Counts pages, finds out which pages have a text layer (now) | Call a language model or hold `ANTHROPIC_API_KEY` |
| OCR and page images (P2-05, later) | Open a network connection of its own |
| Refuses what it cannot read, in a named way | Touch the database or the object store |
| | Decide what an invoice means: validation and every business rule live in `clearcut-core` |

It is the one component that parses hostile files, so it is kept small, secret-free and boxed in: when deployed
(P1-07) it runs as a non-root user on a private network with **no internet egress** and no credentials. A
compromised parser then has nothing to steal and nowhere to send it.

## API (v1)

All errors are JSON with one key: `{"error": "..."}`. Everything except `/health` needs
`Authorization: Bearer <CLEARCUT_WORKER_TOKEN>`; every authentication failure is the same `401`.

| Route | Answer |
|---|---|
| `GET /health` | `200 {"status","version","capabilities"}` (no key needed) |
| `POST /v1/inspect` | Body: the file's bytes. `200 {"media_type":"application/pdf","pages","pages_with_text","has_text_layer"}` |

`/v1/inspect` refuses with: `400` empty body, `401` bad credentials (checked **before** the body is read),
`413` over the size cap (enforced as bytes arrive, whatever length is claimed), `415` not a PDF (judged by the
bytes, never by the caller's `Content-Type`), `422` encrypted, damaged, or over the page cap, `504` too slow.
The worker never repeats anything from the file in an error or a log line, and a caller cannot forge a log line
(control characters in a path are escaped).

**Every inspection runs in a child process** that the worker kills if it overruns the timeout (plus a 2 second
grace), so a hostile file cannot keep the worker busy or crash it. The child reports back in a few bytes of JSON
that are checked field by field and never unpickled. Memory limits belong to the container.

The API description is not served over HTTP. For the contract test it is generated in code:
`create_app(settings).openapi()`.

## Settings (environment)

| Variable | Default | |
|---|---|---|
| `CLEARCUT_WORKER_TOKEN` | **required** | A secret of at least 32 visible characters. The worker will not start without it. |
| `CLEARCUT_WORKER_MAX_BYTES` | 67108864 (64 MiB) | Largest request body |
| `CLEARCUT_WORKER_MAX_PAGES` | 500 | Most pages in a document |
| `CLEARCUT_WORKER_TIMEOUT_SECONDS` | 30 | Give up on a document after this long |
| `CLEARCUT_WORKER_HOST` / `_PORT` | `127.0.0.1` / `8090` | Where to listen. Loopback unless told otherwise. |

## Run and check it (from `services/worker`)

```bash
python -m venv .venv
.venv/Scripts/python -m pip install -r requirements-dev.txt -c constraints.txt   # Windows; use .venv/bin on Linux/macOS
CLEARCUT_WORKER_TOKEN=<a long secret> PYTHONPATH=src .venv/Scripts/python -m clearcut_worker

.venv/Scripts/python -m pytest -q        # tests
.venv/Scripts/python -m ruff check .     # lint
.venv/Scripts/python -m ruff format --check .
.venv/Scripts/python -m mypy             # strict types
```

Dependencies are pinned exactly: direct ones in `requirements.txt` / `requirements-dev.txt`, everything they pull in
in `constraints.txt` (regenerate it from a fresh virtual environment). Needs Python 3.12 or newer; CI runs 3.12 and 3.14.

_Written by Kone Tshivhinda & Claude_
