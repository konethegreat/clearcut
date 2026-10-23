# Written by Kone Tshivhinda & Claude
"""A page's text, where each word is, and what fonts drew it (ADR-0016).

The worker reports **facts**; the Rust core decides what they mean (is this text
layer trustworthy?). The facts here:

- the text of each page, as the PDF's own text layer gives it;
- for each word, where it starts and ends in that text (UTF-8 **byte** offsets,
  the unit the redaction rules use) and the box it occupies on the page (in
  thousandths of the page's width and height from its top left corner, which
  survives drawing the page at any size);
- the fonts the page uses and whether each can say what its characters mean
  (a font with no Unicode map and an unusual encoding is how a text layer comes
  to be gibberish).

The file is hostile. Two parsers read it and must agree on the number of pages
(pypdf for the structure, PDFium for the text and positions); a disagreement
means the document is not trusted. All of this runs inside the isolated child
process (``isolation.py``), never in the worker itself.
"""

from __future__ import annotations

import io
import logging
import math
import time
import unicodedata
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any, Protocol

from pypdf import PdfReader

from .errors import (
    Encrypted,
    InspectError,
    NotAPdf,
    TooManyPages,
    TooSlow,
    Unreadable,
)
from .images import open_image, sniff

log = logging.getLogger("clearcut_worker")

# (left, bottom, right, top) in the PDF's own coordinates, in points.
Box = tuple[float, float, float, float]

_PDF_MAGIC = b"%PDF-"
MAX_PAGE_TEXT_BYTES = 1024 * 1024
MAX_PAGES_PER_CALL = 50
MAX_WORDS_PER_PAGE = 20_000
_MAX_FONTS_PER_PAGE = 64
_MAX_FONT_DICTS_VISITED = 256
_MAX_FORM_DEPTH = 4
_MAX_FORMS_VISITED = 64
_QUARTER_TURNS = (0, 90, 180, 270)


class PageOutOfRange(InspectError):
    """The pages asked for are not all in the document."""

    def __init__(self, pages: int) -> None:
        super().__init__("page range outside the document")
        self.pages = pages


class PageTooLarge(InspectError):
    """A page holds more text or more words than the worker will report."""


@dataclass(frozen=True)
class RawWord:
    """A word found while putting the text together: its place in the text and its box."""

    start: int
    end: int
    box: Box


@dataclass(frozen=True)
class Word:
    start: int
    end: int
    x: int
    y: int
    w: int
    h: int
    # How sure OCR was of the word, in thousandths. A text layer has no such thing.
    confidence: int | None = None


@dataclass(frozen=True)
class FontFact:
    kind: str
    has_to_unicode: bool
    encoding: str
    embedded: bool


@dataclass(frozen=True)
class PageContent:
    page: int
    source: str
    text: str
    width_pt: float
    height_pt: float
    fonts: tuple[FontFact, ...]
    words: tuple[Word, ...] | None
    # Only for an OCR page when asked: per mille of the text layer's words OCR also read.
    agreement: int | None = None


# ------------------------------------------------------- text and word offsets ----


def is_noncharacter(character: str) -> bool:
    code = ord(character)
    return (code & 0xFFFE) == 0xFFFE or 0xFDD0 <= code <= 0xFDEF


def _usable(box: Box | None) -> Box | None:
    """A box only if it has a size; PDF engines give an empty box to characters they invented."""
    if box is None:
        return None
    left, bottom, right, top = box
    if right > left and top > bottom:
        return (float(left), float(bottom), float(right), float(top))
    return None


def assemble_words(raw: str, box_at: Callable[[int], Box | None]) -> tuple[str, list[RawWord]]:
    """Cleans the engine's text and finds the words in it.

    ``raw`` is what the PDF engine returned. ``box_at(i)`` gives the box of the
    character at UTF-16 position ``i`` of ``raw`` (the engine counts in UTF-16
    units, so a character outside the basic plane takes two). Line breaks of any
    kind become a single ``\\n``; control characters and noncharacters are
    dropped. A word is a run of characters that are not white space; its offsets
    are UTF-8 byte positions in the returned text; its box is the union of its
    characters' boxes. A word with no usable box at all cannot be located and is
    left out of the list (it is still in the text).
    """
    pieces: list[str] = []
    words: list[RawWord] = []
    byte_position = 0
    utf16 = 0
    word_start: int | None = None
    word_box: Box | None = None

    def close_word(end: int) -> None:
        nonlocal word_start, word_box
        if word_start is not None and word_box is not None:
            words.append(RawWord(word_start, end, word_box))
        word_start = None
        word_box = None

    index = 0
    while index < len(raw):
        character = raw[index]
        units = 2 if ord(character) > 0xFFFF else 1
        if character == "\r":
            # CRLF is one line break, and still two UTF-16 units of the engine's text.
            if index + 1 < len(raw) and raw[index + 1] == "\n":
                index += 1
                units += 1
            character = "\n"
        elif (
            unicodedata.category(character) == "Cc" and character not in "\n\t"
        ) or is_noncharacter(character):
            utf16 += units
            index += 1
            continue

        if character.isspace():
            close_word(byte_position)
        else:
            if word_start is None:
                word_start = byte_position
            box = _usable(box_at(utf16))
            if box is not None:
                word_box = (
                    box
                    if word_box is None
                    else (
                        min(word_box[0], box[0]),
                        min(word_box[1], box[1]),
                        max(word_box[2], box[2]),
                        max(word_box[3], box[3]),
                    )
                )
        pieces.append(character)
        byte_position += len(character.encode("utf-8"))
        utf16 += units
        index += 1
    close_word(byte_position)
    return "".join(pieces), words


# -------------------------------------------------------------------- geometry ----


def permille_span(low: float, high: float, size: float) -> tuple[int, int]:
    """``(start, length)`` in thousandths of ``size`` for the stretch ``low`` to ``high``.

    Rounded outwards (so it never covers less than what is measured), kept inside
    0 to 1000, and at least one thousandth long.
    """
    start = math.floor(low * 1000 / size + 1e-9)
    end = math.ceil(high * 1000 / size - 1e-9)
    start = max(0, min(start, 999))
    end = max(start + 1, min(end, 1000))
    return start, end - start


@dataclass(frozen=True)
class PageGeometry:
    """The visible area of a page and how it is turned, in the PDF's coordinates.

    PDF measures in points from the bottom left corner of the page *before*
    turning; what a reader sees is the crop box, turned clockwise by the page's
    rotation, measured from the top left. This converts one to the other.
    """

    crop: Box
    rotation: int

    def __post_init__(self) -> None:
        left, bottom, right, top = self.crop
        if not (right > left and top > bottom):
            raise ValueError("the crop box has no area")
        turned = self.rotation % 360
        if turned not in _QUARTER_TURNS:
            raise ValueError("the rotation must be a multiple of 90 degrees")
        object.__setattr__(self, "rotation", turned)

    @property
    def _unturned_width(self) -> float:
        return self.crop[2] - self.crop[0]

    @property
    def _unturned_height(self) -> float:
        return self.crop[3] - self.crop[1]

    @property
    def width_pt(self) -> float:
        return self._unturned_height if self.rotation in (90, 270) else self._unturned_width

    @property
    def height_pt(self) -> float:
        return self._unturned_width if self.rotation in (90, 270) else self._unturned_height

    def _from_top_left(self, x: float, y: float) -> tuple[float, float]:
        """A point of the unturned page (from its bottom left) as seen: from the top left."""
        x -= self.crop[0]
        y -= self.crop[1]
        width, height = self._unturned_width, self._unturned_height
        if self.rotation == 0:
            return x, height - y
        if self.rotation == 90:
            return y, x
        if self.rotation == 180:
            return width - x, y
        return height - y, width - x

    def to_permille(self, box: Box) -> tuple[int, int, int, int]:
        """The box as ``(x, y, width, height)`` in thousandths of the seen page.

        Rounded outwards, so the box never covers less than the glyph, kept inside
        the page, and at least one thousandth in each direction.
        """
        left, bottom, right, top = box
        ax, ay = self._from_top_left(left, bottom)
        bx, by = self._from_top_left(right, top)
        x, width = self._span(min(ax, bx), max(ax, bx), self.width_pt)
        y, height = self._span(min(ay, by), max(ay, by), self.height_pt)
        return x, y, width, height

    @staticmethod
    def _span(low: float, high: float, size: float) -> tuple[int, int]:
        return permille_span(low, high, size)


# ----------------------------------------------------------------------- fonts ----

_KINDS = {
    "/Type1": "type1",
    "/MMType1": "type1",
    "/TrueType": "truetype",
    "/Type0": "type0",
    "/Type3": "type3",
}
_NAMED_ENCODINGS = {
    "/WinAnsiEncoding": "winansi",
    "/MacRomanEncoding": "macroman",
    "/StandardEncoding": "standard",
    "/Identity-H": "identity",
    "/Identity-V": "identity",
}
_FONT_FILE_KEYS = ("/FontFile", "/FontFile2", "/FontFile3")


def _resolve(value: Any) -> Any:
    getter = getattr(value, "get_object", None)
    return getter() if callable(getter) else value


def _is_dictionary(value: Any) -> bool:
    return hasattr(value, "get") and hasattr(value, "keys")


def _describe_font(font: Any) -> FontFact:
    kind = _KINDS.get(str(_resolve(font.get("/Subtype", ""))), "other")
    has_to_unicode = "/ToUnicode" in font
    encoding_value = _resolve(font.get("/Encoding"))
    if encoding_value is None:
        encoding = "none"
    elif _is_dictionary(encoding_value):
        if kind == "type0" or "/Differences" in encoding_value:
            encoding = "custom"
        else:
            base = _resolve(encoding_value.get("/BaseEncoding"))
            encoding = _NAMED_ENCODINGS.get(str(base), "none") if base is not None else "none"
    else:
        encoding = _NAMED_ENCODINGS.get(str(encoding_value), "other")

    descriptor: Any = None
    if kind == "type3":
        embedded = True
    else:
        if kind == "type0":
            descendants = _resolve(font.get("/DescendantFonts"))
            first = _resolve(descendants[0]) if descendants else None
            if first is not None and _is_dictionary(first):
                descriptor = _resolve(first.get("/FontDescriptor"))
        else:
            descriptor = _resolve(font.get("/FontDescriptor"))
        embedded = _is_dictionary(descriptor) and any(key in descriptor for key in _FONT_FILE_KEYS)
    return FontFact(kind=kind, has_to_unicode=has_to_unicode, encoding=encoding, embedded=embedded)


def font_facts(page: Any) -> tuple[FontFact, ...]:
    """The fonts a page can draw with, including those used only inside forms.

    The list is sorted and holds each distinct fact once, so the same fonts always
    give the same answer. A font entry that cannot be read is skipped; the
    walk is bounded in depth and in the number of fonts and forms it visits.
    """
    found: set[FontFact] = set()
    fonts_visited = 0
    forms_visited = 0
    seen_forms: set[int] = set()

    def walk(resources: Any, depth: int) -> None:
        nonlocal fonts_visited, forms_visited
        resources = _resolve(resources)
        if not _is_dictionary(resources):
            return
        fonts = _resolve(resources.get("/Font"))
        if _is_dictionary(fonts):
            for key in list(fonts.keys()):
                if fonts_visited >= _MAX_FONT_DICTS_VISITED:
                    break
                fonts_visited += 1
                font = (
                    _resolve(fonts.raw_get(key))
                    if hasattr(fonts, "raw_get")
                    else _resolve(fonts[key])
                )
                if not _is_dictionary(font):
                    continue
                try:
                    found.add(_describe_font(font))
                except Exception as error:
                    log.debug("a font entry could not be read (%s)", type(error).__name__)
        if depth >= _MAX_FORM_DEPTH:
            return
        objects = _resolve(resources.get("/XObject"))
        if not _is_dictionary(objects):
            return
        for key in list(objects.keys()):
            if forms_visited >= _MAX_FORMS_VISITED:
                break
            reference = objects.raw_get(key) if hasattr(objects, "raw_get") else objects[key]
            identity = getattr(reference, "idnum", None)
            if identity is not None:
                if identity in seen_forms:
                    continue
                seen_forms.add(identity)
            form = _resolve(reference)
            if _is_dictionary(form) and str(_resolve(form.get("/Subtype", ""))) == "/Form":
                forms_visited += 1
                walk(form.get("/Resources"), depth + 1)

    try:
        walk(page.get("/Resources"), 0)
    except Exception as error:
        log.debug("a page's fonts could not be read (%s)", type(error).__name__)
    return tuple(sorted(found, key=lambda f: (f.kind, f.has_to_unicode, f.encoding, f.embedded)))[
        :_MAX_FONTS_PER_PAGE
    ]


# ----------------------------------------------------------------- the document ----


class PdfiumDocument(Protocol):
    """What this module needs of the PDF engine's document (a stand-in is used in tests)."""

    def __len__(self) -> int: ...

    def __getitem__(self, index: int) -> Any: ...

    def close(self) -> None: ...


def open_pdfium(data: bytes) -> PdfiumDocument:
    import pypdfium2

    document: PdfiumDocument = pypdfium2.PdfDocument(data)
    return document


def check_time(deadline: float) -> None:
    if time.monotonic() >= deadline:
        raise TooSlow


def read_structure(data: bytes, *, max_pages: int, deadline: float) -> tuple[PdfReader, int]:
    """Opens a PDF with pypdf and counts its pages (the parser that decides if it is safe).

    Raises the named refusals: not a PDF, too slow, encrypted, unreadable, too many pages.
    """
    if not data.startswith(_PDF_MAGIC):
        raise NotAPdf
    check_time(deadline)
    try:
        reader = PdfReader(io.BytesIO(data), strict=False)
        if reader.is_encrypted:
            raise Encrypted
        count = len(reader.pages)
    except Encrypted:
        raise
    except Exception:
        raise Unreadable from None
    if count > max_pages:
        raise TooManyPages(max_pages)
    return reader, count


def open_engine(
    data: bytes, count: int, pdfium_open: Callable[[bytes], PdfiumDocument]
) -> PdfiumDocument:
    """Opens the same PDF with PDFium; the two parsers must agree on the page count.

    A file that two readers see differently is not one to extract anything from. The
    caller closes the document it gets back.
    """
    try:
        document = pdfium_open(data)
    except Exception:
        raise Unreadable from None
    try:
        agree = len(document) == count
    except Exception:
        agree = False
    if not agree:
        log.debug("the two PDF parsers disagree about the page count")
        document.close()
        raise Unreadable
    return document


def extract_pages(
    data: bytes,
    first: int,
    last: int | None,
    *,
    words: bool,
    max_pages: int,
    deadline: float,
    max_page_text_bytes: int = MAX_PAGE_TEXT_BYTES,
    max_words_per_page: int = MAX_WORDS_PER_PAGE,
    pdfium_open: Callable[[bytes], PdfiumDocument] = open_pdfium,
) -> list[PageContent]:
    """The text, fonts and (if asked) word boxes of pages ``first`` to ``last`` (1-based).

    ``last=None`` means "to the end of the document, but at most ``MAX_PAGES_PER_CALL`` pages".

    Raises the same named refusals as inspection, plus ``PageOutOfRange`` and
    ``PageTooLarge``. Nothing a refusal says comes from the file.
    """
    if first < 1 or (last is not None and last < first):
        raise ValueError("the page range must start at 1 or later and not end before it starts")
    if sniff(data) in ("png", "jpeg"):
        return [_picture_page(data, first, last, words=words, deadline=deadline)]
    reader, count = read_structure(data, max_pages=max_pages, deadline=deadline)
    if first > count or (last is not None and last > count):
        raise PageOutOfRange(count)
    end = min(count, first + MAX_PAGES_PER_CALL - 1) if last is None else last

    document = open_engine(data, count, pdfium_open)
    pages: list[PageContent] = []
    try:
        for number in range(first, end + 1):
            check_time(deadline)
            try:
                pages.append(
                    _read_page(
                        reader.pages[number - 1],
                        document[number - 1],
                        number,
                        words=words,
                        max_page_text_bytes=max_page_text_bytes,
                        max_words_per_page=max_words_per_page,
                    )
                )
            except InspectError:
                raise
            except Exception as error:
                log.debug("page %d could not be read (%s)", number, type(error).__name__)
                raise Unreadable from None
    finally:
        document.close()
    return pages


def _picture_page(
    data: bytes, first: int, last: int | None, *, words: bool, deadline: float
) -> PageContent:
    """A picture is a one-page document: no text layer, no fonts, measured in pixels.

    (The "points" of a picture are its pixels, after it has been turned upright; the
    boxes of a text page are in thousandths of the page, so the unit never matters.)
    """
    check_time(deadline)
    if first != 1 or (last is not None and last != 1):
        raise PageOutOfRange(1)
    width, height = open_image(data).size
    return PageContent(
        page=1,
        source="none",
        text="",
        width_pt=float(width),
        height_pt=float(height),
        fonts=(),
        words=() if words else None,
    )


def _read_page(
    structure: Any,
    engine_page: Any,
    number: int,
    *,
    words: bool,
    max_page_text_bytes: int,
    max_words_per_page: int,
) -> PageContent:
    geometry = PageGeometry(
        crop=tuple(float(v) for v in engine_page.get_cropbox()),  # type: ignore[arg-type]
        rotation=int(engine_page.get_rotation()),
    )
    text_page = engine_page.get_textpage()
    try:
        raw = text_page.get_text_range()
        if len(raw.encode("utf-8", "ignore")) > max_page_text_bytes * 2:
            raise PageTooLarge
        text, found = assemble_words(raw, lambda i: _char_box(text_page, i))
    finally:
        text_page.close()
        engine_page.close()
    if len(text.encode("utf-8")) > max_page_text_bytes or len(found) > max_words_per_page:
        raise PageTooLarge

    if not text.strip():
        return PageContent(
            page=number,
            source="none",
            text="",
            width_pt=geometry.width_pt,
            height_pt=geometry.height_pt,
            fonts=(),
            words=() if words else None,
        )
    located = (
        tuple(Word(w.start, w.end, *geometry.to_permille(w.box)) for w in found) if words else None
    )
    return PageContent(
        page=number,
        source="text_layer",
        text=text,
        width_pt=geometry.width_pt,
        height_pt=geometry.height_pt,
        fonts=font_facts(structure),
        words=located,
    )


def _char_box(text_page: Any, index: int) -> Box | None:
    try:
        left, bottom, right, top = text_page.get_charbox(index, loose=True)
    except Exception:
        return None
    return (float(left), float(bottom), float(right), float(top))
