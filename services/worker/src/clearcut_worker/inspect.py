# Written by Kone Tshivhinda & Claude
"""Looking inside a PDF without trusting it.

The input is a hostile file: it may be damaged, encrypted, enormous, or crafted
to exploit a parser. This module keeps to what is cheap and safe (read the page
tree, ask each page for its text) and gives up in a named way when it cannot.
Error messages state which rule failed and never repeat anything from the file.
"""

from __future__ import annotations

import io
import logging
import time
from dataclasses import dataclass

from pypdf import PdfReader

log = logging.getLogger("clearcut_worker")

_PDF_MAGIC = b"%PDF-"


class InspectError(Exception):
    """Something stops the document from being inspected."""


class NotAPdf(InspectError):
    """The bytes do not start like a PDF."""


class Unreadable(InspectError):
    """The file claims to be a PDF but cannot be read (damaged, or not valid)."""


class Encrypted(InspectError):
    """The PDF is password-protected. We never try passwords."""


class TooManyPages(InspectError):
    def __init__(self, max_pages: int) -> None:
        super().__init__("too many pages")
        self.max_pages = max_pages


class TooSlow(InspectError):
    """The deadline passed before inspection finished."""


@dataclass(frozen=True)
class Inspection:
    pages: int
    # How many pages have any text at all in their text layer. A scan has 0.
    pages_with_text: int

    @property
    def has_text_layer(self) -> bool:
        return self.pages_with_text > 0


def inspect_pdf(data: bytes, max_pages: int, deadline: float) -> Inspection:
    """Counts the pages and the pages that carry text.

    ``deadline`` is a ``time.monotonic()`` instant; the work is abandoned between
    pages once it has passed. (A single page that takes forever cannot be
    interrupted from here; the container's CPU and time limits cover that.)
    """
    if not data.startswith(_PDF_MAGIC):
        raise NotAPdf
    _check_time(deadline)

    try:
        reader = PdfReader(io.BytesIO(data), strict=False)
        if reader.is_encrypted:
            raise Encrypted
        page_count = len(reader.pages)
    except Encrypted:
        raise
    except Exception:
        # Parsers raise many kinds of error on hostile input. All of them mean
        # "cannot be read", and none of their messages may reach the caller.
        raise Unreadable from None

    if page_count > max_pages:
        raise TooManyPages(max_pages)

    with_text = 0
    for index in range(page_count):
        _check_time(deadline)
        try:
            text = reader.pages[index].extract_text()
        except Exception as error:
            # A page whose text cannot be pulled out counts as having none; the
            # OCR path (P2-05) is for pages like that. Only the kind of error is
            # logged: its message could quote the document.
            log.debug("page %d: text extraction failed (%s)", index + 1, type(error).__name__)
            continue
        if text.strip():
            with_text += 1
    return Inspection(pages=page_count, pages_with_text=with_text)


def _check_time(deadline: float) -> None:
    if time.monotonic() >= deadline:
        raise TooSlow
