# Written by Kone Tshivhinda & Claude
"""Painting boxes black on a picture, and nothing else (ADR-0016, ADR-0014).

``clearcut-core`` decides which words must be hidden before a page is sent to a model, and
works out where they are. This module has **no rules**: it is given rectangles and paints
them solid black. That keeps what counts as personal data in one place, and keeps the
worker, which handles hostile files, free of business logic.

A box is ``(x, y, w, h)`` in thousandths of the picture from its top left, the same unit as
the word boxes the text and OCR endpoints report, so a box taken from there can be sent as
it is. It is turned into whole pixels **rounded outwards**, so it never covers less than
it was asked to. Any safety margin around a word is for the caller to add.

What comes back is built from pixels only (see ``images.py``): nothing that was under a box
survives in the result, and neither does any metadata of the picture it came from.
"""

from __future__ import annotations

import logging
from collections.abc import Sequence

from PIL import Image, ImageDraw

from .images import (
    DEFAULT_MAX_EDGE,
    MAX_IMAGE_BYTES,
    MAX_IMAGE_PIXELS,
    EncodedImage,
    encode_image,
    open_image,
)
from .pagetext import check_time

log = logging.getLogger("clearcut_worker")

MAX_BOXES = 500
_BLACK = (0, 0, 0)

# (x, y, width, height), each in thousandths of the picture.
PermilleBox = tuple[int, int, int, int]
# (left, top, right, bottom) in pixels; right and bottom are one past the last pixel.
Rectangle = tuple[int, int, int, int]


def check_box(box: object) -> PermilleBox:
    """The box, if it is four whole numbers describing a stretch inside the picture."""
    if not isinstance(box, tuple | list) or len(box) != 4:
        raise ValueError("a box must be four whole numbers: x, y, w, h")
    if not all(isinstance(n, int) and not isinstance(n, bool) for n in box):
        raise ValueError("a box must be four whole numbers: x, y, w, h")
    x, y, w, h = box
    if x < 0 or y < 0 or w < 1 or h < 1 or x + w > 1000 or y + h > 1000:
        raise ValueError("a box must lie inside the picture and have some size")
    return x, y, w, h


def pixel_rectangle(box: PermilleBox, size: tuple[int, int]) -> Rectangle:
    """The pixels a box covers, rounded outwards and always at least one pixel."""
    x, y, w, h = box
    width, height = size
    return (
        x * width // 1000,
        y * height // 1000,
        -(-(x + w) * width // 1000),
        -(-(y + h) * height // 1000),
    )


def paint(picture: Image.Image, boxes: Sequence[PermilleBox]) -> None:
    """Paints the boxes black on the picture, in place."""
    draw = ImageDraw.Draw(picture)
    for box in boxes:
        left, top, right, bottom = pixel_rectangle(box, picture.size)
        draw.rectangle((left, top, right - 1, bottom - 1), fill=_BLACK)


def mask_image(
    data: bytes,
    boxes: Sequence[object],
    *,
    deadline: float,
    fmt: str = "png",
    max_edge: int = DEFAULT_MAX_EDGE,
    max_bytes: int = MAX_IMAGE_BYTES,
    max_pixels: int = MAX_IMAGE_PIXELS,
) -> EncodedImage:
    """The PNG or JPEG ``data`` with the ``boxes`` painted black.

    Raises ``ValueError`` for a box that cannot be right (the caller checks them first),
    and the same named refusals as drawing a page for the picture itself.
    """
    if len(boxes) > MAX_BOXES:
        raise ValueError(f"at most {MAX_BOXES} boxes can be painted at once")
    checked = [check_box(box) for box in boxes]
    check_time(deadline)
    # (Anything that is not a PNG or a JPEG is refused by open_image, as unsupported.)
    picture = open_image(data, max_pixels=max_pixels)
    check_time(deadline)
    paint(picture, checked)
    return encode_image(picture, fmt=fmt, max_edge=max_edge, max_bytes=max_bytes)
