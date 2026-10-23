# ClearCut document worker

The Python half of document processing ([ADR-0009](../../docs/adr/0009-worker-and-provider-boundary.md)). It
**reads untrusted documents** and tells the Rust service what is in them. Nothing else.

| It does | It never does |
|---|---|
| Counts pages, finds out which pages have a text layer | Call a language model or hold `ANTHROPIC_API_KEY` |
| Reads each page's text, word boxes and font facts (now) | Open a network connection of its own |
| Draws page images and re-encodes PNG and JPEG documents (now), always as plain pixels with no metadata | Judge whether a page's text can be trusted: it reports facts, `clearcut-core` decides |
| Paints boxes black on a picture, and returns only pixels (the image masker of ADR-0014) | |
| OCR with Tesseract in English and Afrikaans, and the agreement between OCR and the text layer (when Tesseract is installed and named) | |
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
| `POST /v1/mask?format=png&max_edge=2000` | Body: JSON `{"image": "<base64 PNG or JPEG>", "boxes": [{"x","y","w","h"}]}`. `200` the picture with the boxes painted black (`image/png` or `image/jpeg`), size in `X-Image-Width` and `X-Image-Height` |
| `POST /v1/ocr?page=1&agreement=false` | Body: the file's bytes. `200` the text answer with one page, `source` `ocr`, a `confidence` (0 to 1000) on every word, and `agreement` if asked. `503` when OCR is not set up |
| `POST /v1/render?page=1&dpi=150&format=png&max_edge=2000` | Body: the file's bytes. `200` the image (`image/png` or `image/jpeg`), size in `X-Image-Width` and `X-Image-Height` |
| `POST /v1/text?from=1&to=50&words=false` | Body: the file's bytes. `200 {"pages":[{"page","source","text","width_pt","height_pt","fonts":[{"kind","has_to_unicode","encoding","embedded"}],"words":[{"start","end","x","y","w","h"}]}]}` |

`/v1/mask` paints boxes solid black on a PNG or JPEG and gives the picture back. A box is `{"x","y","w","h"}` in thousandths of the
picture from its top left (the unit of the word boxes in the text and OCR answers), at most 500 of them, each inside the picture,
whole numbers only. They are rounded outwards to whole pixels. The worker has **no rules**: `clearcut-core` decides which words
to hide and how much margin to leave, and sends rectangles. The picture is rebuilt from pixels, so nothing under a box, and no
metadata or transparency, is in the result; a photo is turned upright first. The base64 must be plain, the picture at most 16 MiB
(`413` otherwise), `415` for any other file type, `422` for a damaged or over-large picture.

`/v1/ocr` reads one page (or a PNG/JPEG document) with Tesseract, in English and Afrikaans together. The page is drawn at 300 dpi
for it. The answer has the shape of the `/v1/text` answer: one page with `source: "ocr"`, `fonts: []`, the text (words joined by a
space, lines by a line feed), and every word with its UTF-8 byte offsets, its box in thousandths of the page and its `confidence`.
With `agreement=true` it also gives `agreement`: of the distinct words of two letters or digits or more in the PDF's own text
layer, how many thousandths OCR also read (left out when the page has no text layer). A text layer that lies, such as a scrambled
font or digits swapped for other digits, shares few of its words with what a reader sees; **what a low number means is decided by
`clearcut-core`, not here**. It refuses like `/v1/text`, and `503` when OCR is not set up. OCR is off unless
`CLEARCUT_WORKER_TESSERACT` names the program; the worker then checks at start that it runs and has both languages and **will not
start otherwise**. Tesseract runs with a fixed argument list, no shell, in the child's scratch directory, with nothing secret in its
environment and under the request's deadline.

`/v1/render` draws one page (or a picture document) as an image. Query: `page` (default 1), `dpi` (100 to 300, default
150; meaningless for a picture), `format` (`png` default, or `jpeg`), `max_edge` (the longest side in pixels, 500 to 2 576,
default 2 000). The answer is the image bytes (`image/png` or `image/jpeg`) with `X-Image-Width`, `X-Image-Height` and
`Cache-Control: no-store`. The image is always rebuilt from pixels, so EXIF, GPS, colour profiles and text chunks never
leave; a photo is turned upright first and transparency becomes white. A PNG over 5 MiB is sent as a JPEG, and a picture
that still does not fit is made smaller (and refused below a long edge of 100). Forms in a PDF are not drawn. It refuses
like `/v1/text`, and also `422` `{"error":"the page is too large to render"}` (over 14 400 points on a side, or over 40
million pixels at that dpi) and `422` `{"error":"the image is too large"}` (a picture over 40 million pixels, decided
from its header before it is decoded).

`/v1/inspect` and `/v1/text` also accept a PNG or a JPEG: it is a one-page document with no text layer
(`media_type` `image/png` or `image/jpeg`), and `/v1/text` measures its one page in pixels. Only these two picture
formats are read.

`/v1/text` reads pages `from` to `to` (1-based, at most 50 in one call; leaving `to` out reads to the end of the
document but never more than 50 pages). `source` is `text_layer`, or `none` when the page has no text (then `text` is
empty and `fonts` is `[]`). `text` is cleaned: line breaks become `\n`, control characters and noncharacters are dropped.
`width_pt` and `height_pt` are the page as a reader sees it (crop box, turned by the page's rotation). `fonts` lists each
distinct font the page can draw with, including those inside forms: `kind` (`type1`, `truetype`, `type0`, `type3`,
`other`), `encoding` (`none`, `winansi`, `macroman`, `standard`, `identity`, `custom`, `other`), whether it has a
`ToUnicode` map and whether its font file is embedded. These are **facts for the text-quality verdict, which is decided
in `clearcut-core`, not here**. `words` appears only with `words=true`: `start`/`end` are **UTF-8 byte offsets** into `text`,
and `x`, `y`, `w`, `h` a box in thousandths of the page from its top left, rounded outwards (so a box never covers less
than the glyphs) and made from the whole line height, so a block painted over it hides descenders too.
The worker checks that two PDF readers (pypdf and PDFium) agree on the page count and refuses the document as
unreadable when they do not.

`/v1/text` refuses like `/v1/inspect`, and also: `400 {"error":"invalid request"}` for a bad range or flag
(`from` < 1, `to` < `from`, more than 50 pages, anything not a number or a boolean), `422` with
`{"error":"the page range is outside the document","pages":N}` when the range runs past the end, and `422` when one
page holds more than 1 MiB of text or 20 000 words (refused, never cut short).

`/v1/inspect` refuses with: `400` empty body, `401` bad credentials (checked **before** the body is read),
`413` over the size cap (enforced as bytes arrive, whatever length is claimed), `415` not a PDF (judged by the
bytes, never by the caller's `Content-Type`), `422` encrypted, damaged, or over the page cap, `504` too slow.
The worker never repeats anything from the file in an error or a log line, and a caller cannot forge a log line
(control characters in a path are escaped).

**Every document is handled in a child process** that the worker kills if it overruns the timeout (plus a 2 second
grace), so a hostile file cannot keep the worker busy or crash it. The child:

- is **started with a scrubbed environment** (only a short allowlist survives; the worker's other variables are set aside
  for the instant the child is created), so it cannot read the bearer token or any other secret, not even from the block
  of environment it started with, and neither can a program it starts;
- works in its own **scratch directory**, deleted afterwards even if the child was killed;
- runs under **limits** set before it touches the file (Linux; Windows has none, so there the container is the limit):
  memory, processor time, open files, the size of any file it writes, and no core dumps. A limit that cannot be set
  stops the child before it opens the document;
- reports back **bytes** with a size cap (a few hundred bytes for an inspection, up to 32 MiB for the text of a batch of pages, and megabytes for page images later) that
  are checked by the parent and never unpickled.

The container (P1-07) adds the rest: non-root user, read-only file system, no network, no credentials.

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
| `CLEARCUT_WORKER_TESSERACT` | unset (OCR off) | The Tesseract program (a name on the path, or a full path). Set it to turn OCR on; the worker will not start if it cannot be used or lacks English or Afrikaans. |
| `CLEARCUT_WORKER_TESSDATA` | unset | Where Tesseract's language data is, if not where it looks (only with the setting above). |

## Licences

ClearCut is Apache-2.0, so the worker may hold **permissively licensed code only** (MIT, BSD, Apache-2.0, ISC, PSF, HPND and a few
similar). A GPL, LGPL or AGPL package, or one whose licence cannot be read, fails CI. `python tools/licences.py` reads what each
installed package says about itself, follows the packages in `requirements.txt` (not the test tools), and refuses the rest.
A package that states its licence in words the rules cannot read needs an entry in `licence-exceptions.toml` with the exact
wording and a reason; the entry stops working if the wording changes and is reported once the package is no longer shipped
([ADR-0016](../../docs/adr/0016-page-text-images-and-text-quality.md)).

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
