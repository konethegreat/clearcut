# Written by Kone Tshivhinda & Claude
"""Drawing a page (or opening a picture) as the image the model will be shown (ADR-0016).

A PDF page is drawn by PDFium at the dpi asked for; a PNG or JPEG document is its own
single page. Either way the result goes through ``images.encode_image``, so what leaves
is plain pixels in a PNG (or a JPEG when the PNG is too big): no metadata, no colour
profile, nothing but the picture.

The sizes are checked **before** anything is drawn: a page more than 14 400 points on a
side, or one that would need more than 40 million pixels at the dpi asked for, is
refused, because drawing it would use memory the worker does not have.

This runs only inside the isolated child process (``isolation.py``).
"""

from __future__ import annotations

import logging
import math
from collections.abc import Callable

from .errors import InspectError, Unreadable
from .images import (
    DEFAULT_MAX_EDGE,
    MAX_IMAGE_BYTES,
    MAX_IMAGE_PIXELS,
    EncodedImage,
    encode_image,
    open_image,
    sniff,
)
from .pagetext import (
    PageOutOfRange,
    PdfiumDocument,
    check_time,
    open_engine,
    open_pdfium,
    read_structure,
)

log = logging.getLogger("clearcut_worker")

MIN_DPI = 100
DEFAULT_DPI = 150
MAX_DPI = 300
# PDF's own upper limit for a page is 14 400 units on a side (200 inches).
MAX_PAGE_POINTS = 14_400


class PageTooBig(InspectError):
    """The page would be too big to draw: too long on a side, or too many pixels."""


def render_page(
    data: bytes,
    page: int,
    *,
    dpi: float = DEFAULT_DPI,
    fmt: str = "png",
    max_edge: int = DEFAULT_MAX_EDGE,
    max_bytes: int = MAX_IMAGE_BYTES,
    max_pages: int,
    deadline: float,
    max_pixels: int = MAX_IMAGE_PIXELS,
    pdfium_open: Callable[[bytes], PdfiumDocument] = open_pdfium,
) -> EncodedImage:
    """Page ``page`` (1-based) of the document as an image.

    Raises the same named refusals as reading text, plus ``PageTooBig`` for a PDF page
    and ``ImageTooLarge`` for a picture. Nothing a refusal says comes from the file.
    """
    if page < 1:
        raise ValueError("the page must be 1 or later")
    if sniff(data) in ("png", "jpeg"):
        check_time(deadline)
        if page != 1:
            raise PageOutOfRange(1)
        picture = open_image(data, max_pixels=max_pixels)
        check_time(deadline)
        return encode_image(picture, fmt=fmt, max_edge=max_edge, max_bytes=max_bytes)

    _reader, count = read_structure(data, max_pages=max_pages, deadline=deadline)
    if page > count:
        raise PageOutOfRange(count)
    document = open_engine(data, count, pdfium_open)
    try:
        check_time(deadline)
        try:
            picture = _draw(document[page - 1], dpi, max_pixels)
        except InspectError:
            raise
        except Exception as error:
            log.debug("page %d could not be drawn (%s)", page, type(error).__name__)
            raise Unreadable from None
    finally:
        document.close()
    check_time(deadline)
    return encode_image(picture, fmt=fmt, max_edge=max_edge, max_bytes=max_bytes)


def _draw(engine_page: object, dpi: float, max_pixels: int):  # type: ignore[no-untyped-def]
    """Draws one page with PDFium and returns it as a picture."""
    try:
        width_pt, height_pt = engine_page.get_size()  # type: ignore[attr-defined]
        if not (math.isfinite(width_pt) and math.isfinite(height_pt)):
            raise Unreadable
        if width_pt <= 0 or height_pt <= 0:
            raise Unreadable
        if width_pt > MAX_PAGE_POINTS or height_pt > MAX_PAGE_POINTS:
            raise PageTooBig
        scale = dpi / 72
        if math.ceil(width_pt * scale) * math.ceil(height_pt * scale) > max_pixels:
            raise PageTooBig
        # Forms are not drawn: there is nothing in an invoice's image that needs them,
        # and a form is one more thing in the file that can run.
        bitmap = engine_page.render(scale=scale, may_draw_forms=False)  # type: ignore[attr-defined]
        return bitmap.to_pil()
    finally:
        engine_page.close()  # type: ignore[attr-defined]
