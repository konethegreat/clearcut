# ADR-0016: Page text, page images and text quality

Status: **Proposed** · Date: 2026-10-23 · Deciders: Kone Tshivhinda, Claude
Task P2-05, with the **detection half of P2-20** (glyph-scrambled PDFs) and the worker half of the image masker promised in [ADR-0014](0014-context-pack-builder.md). Builds on [ADR-0009](0009-worker-and-provider-boundary.md) (what the worker is and is not), [ADR-0014](0014-context-pack-builder.md) (what a pack needs per page), [ADR-0013](0013-model-facing-reading-shape.md), and [research 06](../research/06-anthropic-api.md) §7 (images). Licences were read on PyPI and the projects' pages on 23 Oct 2026. Design only; nothing is built until this is approved. P2-20 asks for a spike on real samples before its design is fixed: this ADR fixes only how a bad text layer is **noticed**, with thresholds marked provisional until real samples exist.

## What it is
Today the worker answers one question: how many pages, and which have any text. The context pack (ADR-0014) needs, for each page, a **picture**, the **text** with a note of where it came from, and (for hiding personal data in the picture) **where each word is**. And the whole pipeline needs to know when a text layer **lies**: a PDF whose fonts scramble the glyph-to-character mapping has a text layer that exists and is gibberish, so "has a text layer" is not good enough.

```text
file ─► worker (hostile input, no keys, no internet) ─► facts: text, word boxes, picture, font facts, OCR
                                                        └─► Rust core decides: is this text good? (text_quality)
```

The rule that shapes the design: **the worker reports facts and does pixel and text work; `clearcut-core` decides what they mean.** Thresholds are business rules, they will be tuned on the gold set, and they should not need a Python release to change.

## 1. Libraries, and a correction to ADR-0009
ADR-0009 named PyMuPDF. **PyMuPDF is AGPL-3.0 (or a paid commercial licence)**; an AGPL dependency would make every deployment of ClearCut subject to it and is not acceptable for an Apache-2.0 project meant to be freely used. It is replaced. The rule from now on: **permissive licences only (Apache-2.0, BSD, MIT, HPND); no GPL, LGPL-linked or AGPL code in the worker image**, checked in CI.

| Job | Library | Licence | Notes |
|---|---|---|---|
| Page count, encryption, fonts of a page | **pypdf** (already used) | BSD-3 | Pure Python, so the part that decides "is this document safe to open" is memory-safe |
| Text with character boxes, rendering | **pypdfium2** (PDFium) | BSD-3-Clause / Apache-2.0, plus the licences of what PDFium bundles | PDFium is Chrome's PDF engine; native code, so it only ever runs in the isolated child (§5). The standard wheels are built without the JavaScript engine (to be confirmed when pinning) |
| Resize, PNG/JPEG, strip metadata | **Pillow** | HPND | Decompression-bomb limit set explicitly |
| OCR | **Tesseract** (the program), with the `eng` and `afr` language data | Apache-2.0 (Leptonica: BSD-2) | Run as a program with fixed arguments, not through a wrapper library; **Afrikaans is included because South African documents often are, or are bilingual** |
| Test fixtures only | **reportlab** (BSD), **fontTools** (MIT) | | Make PDFs with known, deliberately scrambled fonts at test time |

Rejected: **PyMuPDF** (AGPL), **Ghostscript** (AGPL) and anything that needs it (OCRmyPDF's PDF paths), **poppler/pdftotext** (GPL), `pytesseract` (a thin wrapper around the same program; one more dependency for no gain), heavy neural OCR (EasyOCR, PaddleOCR, docTR) and Docling for now (large images; an option if Tesseract proves too weak on the gold set). `pdfminer.six` (MIT) is kept in reserve for font analysis if pypdf's view of fonts is not enough.

## 2. The worker's API (v1 additions)
Same hardening as `/v1/inspect`: bearer token checked before the body is read, size cap as bytes arrive, the file type from the bytes, errors that never repeat the file, one child process per request that is killed on overrun. All additions are **stateless**: the file is sent with each call. That re-sends a few megabytes over the private network per page, and avoids the worker keeping other people's documents between calls. If measurement shows it matters, a short-lived spool can be added later by its own decision.

| Route | Body and query | Answer |
|---|---|---|
| `POST /v1/text` | file; `from`, `to` (at most 50 pages per call); `words=true` for boxes | `200` JSON, one entry per page: `page`, `source` (`text_layer` or `none`), `text`, `width_pt`, `height_pt`, `fonts` (§4), and with `words`, `words: [{start, end, x, y, w, h}]` |
| `POST /v1/ocr` | file; `page` | the same shape with `source: "ocr"`, plus a per-word `confidence` (0 to 1000) |
| `POST /v1/render` | file; `page`; `dpi` (100 to 300, default 150); `format` (`png` default) | the image bytes (`image/png` or `image/jpeg`), with `X-Image-Width` and `X-Image-Height` |
| `POST /v1/mask` | a PNG; `boxes` (up to 500) | the PNG with those boxes painted solid black |
| `POST /v1/inspect` | file | now also accepts PNG and JPEG (a phone photo of an invoice is a one-page document) |

- **Boxes** are in **per-mille of the page's width and height**, top-left origin, the same as `Locator::BoundingBox`, so they survive re-rendering at any size. `start` and `end` are **UTF-8 byte offsets into `text`**, the same unit the redaction spans use, so Rust can turn a redaction span into boxes without converting anything (tests check this with accented text and an emoji).
- **`/v1/mask` has no rules in it:** Rust decided which words to hide and sends the rectangles; the worker only paints. This is the real `ImageMasker` ADR-0014 waited for.
- **Limits:** 500 pages (existing), one page's text at most 1 MiB, a page larger than 14 400 points on a side or more than 40 million pixels at the asked-for dpi is refused before rendering, an image document over 40 million pixels is refused before it is decoded.

## 3. Page images (what the model sees)
- **Defaults:** 150 dpi, so an A4 page is about 1 240 x 1 754 px, roughly **2 800 tokens** (research 06 §7). PNG, because lossy compression hurts small print. If the PNG is over **5 MiB** (our own cap, below the API's 10 MB) it is re-encoded as JPEG at quality 90, and only if still too big is it scaled down. The long edge never exceeds **2 000 px** by default (the API takes up to 2 576 on the models we use); a caller can ask for more dpi for a dense scan, within the limits above.
- **Metadata is dropped:** the image is always re-encoded, so EXIF (GPS position, camera, time) from a phone photo never leaves.
- **Orientation:** a photo is rotated upright from its EXIF orientation before the metadata is dropped. Detecting a sideways scan is not attempted here.
- Fonts that PDFium cannot find are substituted, which can change the look but not the content of the page.

## 4. Telling that a text layer lies (the detection half of P2-20)
Two kinds of evidence, one from each side of the boundary.

**Facts the worker reports** (cheap, structural; from pypdf on the page's resources):
- per font used on the page: `kind` (`type1`, `truetype`, `type0`, `type3`, `other`), `has_to_unicode`, `encoding` (`winansi`, `macroman`, `standard`, `identity`, `custom`, `none`), `embedded`;
- on request, an **OCR agreement**: render the page, OCR it, and report the share (per-mille) of the text layer's alphanumeric tokens that also appear in the OCR output. This is the only check that also catches **digits that are permuted but still look like digits**, which no look at the text alone can.

**What Rust computes from the text itself** (pure, in a new `clearcut_core::text_quality`, with fixtures): the share of replacement, private-use and control characters; the share of letters, digits and punctuation; the share of words of three or more letters with **no vowel**; unlikely mid-word capitals; very long unbroken tokens; and **invoice keyword hits** from a short English and Afrikaans list (invoice, tax, vat, total, date, amount, quantity, price, description, due; faktuur, btw, totaal, datum, bedrag, hoeveelheid, prys, beskrywing, vervaldatum). The list is a rule, so it lives in core.

**The verdict** (`Good`, `NeedsCheck`, `Unusable`) comes from core, with provisional thresholds:
- `Unusable`: no text at all; or 5% or more replacement or private-use characters; or an OCR agreement below 50%.
- `NeedsCheck`: a risky font (Type 3, or a font with a custom or identity encoding and no Unicode map) **and** weak text (no keywords, or many vowelless words); or an OCR agreement from 50% to 85%.
- `Good`: otherwise, and also an OCR agreement of 85% or more.

**What each verdict does:**
- `Good`: the text layer goes in the pack as `text_layer`.
- `NeedsCheck`: run the OCR agreement check, which makes it `Good` or `Unusable`.
- `Unusable`: **the text layer is not used**; the page's **OCR text** goes in the pack as `ocr` (so the model still gets text, the redaction rules still have text to check, and the masker still has boxes), and the page is recorded as `text_layer_rejected`.
- **Page 1 of every PDF always gets the OCR agreement check**, as a cheap sentinel for a scrambled document; whether every page should is a question for the gold set.

**What this deliberately does not claim.** Detection reports evidence, not certainty. A scrambled font that maps letters to other letters is usually caught; one that only permutes digits looks like valid text. The defence is the layering that already exists: the model reads the **image**, ADR-0014's rules tell it to trust the image over the text, `validate` re-derives every total to the cent, and the cross-check of P2-13c confirms that the amounts the model read appear in the text layer (a scrambled layer then **disagrees** and the page goes to review). Nothing in this ADR reconstructs a scrambled font's glyph map (P2-20 step 3); that waits for real samples.

## 5. Isolation
The existing child-process design (spawned fresh interpreter, hard time limit with a grace period, crashes stay in the child, answers checked, nothing unpickled) stays. What changes:
- **Bigger answers.** A page's text or image is megabytes, not 256 bytes. The child writes its answer to a file in a **per-request temporary directory** (deleted afterwards) and sends the parent only a small status message; the parent reads the file with a hard size cap. JSON is parsed with a size and depth limit; image bytes are only length-checked and given the right magic bytes, never parsed by the parent.
- **Limits inside the child** on Linux, before it touches the file: `RLIMIT_AS` (address space, 2 GiB by default), `RLIMIT_CPU`, `RLIMIT_NOFILE`, no core dumps. The container supplies the rest (P1-07): non-root user, read-only root file system, no network, no credentials.
- **Tesseract** is run from the child as a program with a fixed list of arguments (no shell), the page image on its input, the result read from its output, a time limit of its own, an empty environment apart from the language-data path, and the temporary directory as its working directory.
- A **parser disagreement is a refusal:** pypdf and PDFium must report the same page count, else the document is `Unreadable`.
- Windows development: the resource limits and OCR tests are skipped when the platform cannot do them; CI on Linux runs them all.

## 6. The Rust side
- `clearcut-docproc`: `DocumentProcessor` gains `page_texts`, `ocr_page`, `render_page` and `mask_image`; new types `PageContent`, `Word`, `FontFacts`, `RenderedPage`; `HttpWorker` and `FakeWorker` follow, with the same error mapping to job failure kinds. A `WorkerMasker` implements ADR-0014's `ImageMasker` by turning redaction spans into boxes through the words and calling `mask_image`.
- `clearcut-core`: `text_quality` (measures, verdict, reasons as fixed words; no text in any reason).
- The pack builder is unchanged: it already takes `PageText { source, content }` and an `ImageMasker`.

## 7. Tests (no key, no real invoice)
- **Generated at test time:** a normal text PDF; a scan (text drawn as an image only); a PDF whose font is a subset with a **custom encoding and no Unicode map**; one with a **Type 3** font; one that **permutes digits only**; a page with accented text and an emoji (for the byte offsets); an encrypted PDF; a tiny page; a huge page (refused); a decompression-bomb PNG (refused); a PNG with EXIF (stripped).
- The 15 synthetic gold-set PDFs: `/v1/text` finds each label's invoice number, date and amounts on the labelled page, and the whole set is `Good`.
- Boxes: for every sample word, the box lies inside the page, and masking a box makes those pixels black and leaves a neighbouring region unchanged.
- The text-quality measures and verdicts have unit tests in core with the generated texts as fixtures; thresholds are constants with a comment saying they are provisional.
- Hostile input: truncated, wrong-type and oversized files; a child that hangs, crashes or answers with oversized or malformed data; errors that never contain file content.
- OCR tests need Tesseract (`eng`, `afr`): the CI job installs it; locally they are skipped when it is absent.
- CI also checks the worker's **dependency licences** against an allowlist, so an AGPL or GPL package cannot slip in.

## Build order (after approval)
1. Licence check in CI, and the isolation changes (file-based answers, limits). 2. `/v1/text` with boxes and fonts. 3. `/v1/render` and image documents. 4. `/v1/ocr` and the OCR agreement. 5. `/v1/mask`. 6. `text_quality` in core. 7. The Rust trait, client, fake and `WorkerMasker`. 8. Mutation check, docs, PR for each (small PRs).

## Alternatives considered
- *Keep PyMuPDF.* Best-in-class, but AGPL; rejected for an Apache-2.0 project.
- *Send the PDF straight to Claude.* The API accepts PDFs, but it gives the model the raw file, gives us no say about scrambled text layers, and defeats redaction (research 06 §7).
- *Decide the verdict in the worker.* Faster to write, but then thresholds and word lists are business rules in the wrong language and the wrong repository layer.
- *OCR every page every time.* The strongest independent read, at a CPU cost on every page of every document; the vision read plus the P2-13c cross-check already give an independent check, so OCR is used where there is evidence of trouble and on page 1 as a sentinel.
- *A stateful worker that keeps uploaded files.* Fewer bytes on the wire, but it keeps other people's documents between calls; deferred until measured.

## Questions for Kone
1. **Replace PyMuPDF with pypdfium2** (plus pypdf and Pillow), call **Tesseract** (`eng` and `afr`) directly, and enforce **permissive licences only** with a CI check: agreed?
2. **Stateless endpoints** (the file is re-sent per call, text in batches of up to 50 pages): agreed for now?
3. **The worker reports facts; `clearcut-core` decides the text-quality verdict,** with provisional thresholds until your real obfuscated samples allow tuning: agreed?
4. **Image policy:** 150 dpi, PNG, JPEG fallback under 5 MiB, long edge at most 2 000 px, metadata always dropped, PNG and JPEG accepted as documents: agreed?
5. **Word boxes in per-mille, UTF-8 byte offsets, and a paint-only `/v1/mask`** so the image masker becomes real: agreed?
6. **OCR agreement check on page 1 of every PDF and on every `NeedsCheck` page:** agreed?
7. **Fixtures generated at test time** (reportlab and fontTools as test-only dependencies) and Tesseract installed in CI: agreed? (Real obfuscated samples from you are wanted for the P2-20 spike but do not block this.)
