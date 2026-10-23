# Written by Kone Tshivhinda & Claude
"""Pictures: opening a PNG or JPEG that may be hostile, and encoding one to send back (ADR-0016).

What a caller gets back is always built from pixels only. A phone photo carries its
position, camera and time in EXIF; a scan may carry a colour profile or notes. None
of that is ever passed on, because the picture is decoded to plain RGB pixels and
encoded again from scratch.

Only PNG and JPEG are opened (Pillow can read dozens of formats, and every extra
decoder is more hostile-input code to trust). The size is read from the header and
checked **before** anything is decoded, so a few bytes that claim to be a
30 000 by 30 000 picture cost nothing.
"""

from __future__ import annotations

import io
import logging
import math
from dataclasses import dataclass

from PIL import Image, ImageOps

from .errors import InspectError, NotAPdf, Unreadable

log = logging.getLogger("clearcut_worker")

# A picture bigger than this is refused (ADR-0016 section 2): 40 million pixels is
# about a 6 300 by 6 300 scan, and costs about 120 MB once decoded.
MAX_IMAGE_PIXELS = 40_000_000
# Our own limit on what is sent back; the model API's is 10 MB (research 06 section 7).
MAX_IMAGE_BYTES = 5 * 1024 * 1024
# The longest side of a picture sent back, unless the caller asks for more.
DEFAULT_MAX_EDGE = 2000
# The API takes up to 2 576 px on the models ClearCut uses.
MAX_EDGE_LIMIT = 2576
# When a picture must be made smaller to fit, it is never made smaller than this.
_SMALLEST_LONG_EDGE = 100
_JPEG_QUALITY = 90

# Pillow's own protection stays on as well: it refuses at twice this number.
Image.MAX_IMAGE_PIXELS = MAX_IMAGE_PIXELS

_PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
_JPEG_SIGNATURE = b"\xff\xd8\xff"
_PDF_MAGIC = b"%PDF-"
_WHITE = (255, 255, 255)


class ImageTooLarge(InspectError):
    """A picture has more pixels than the worker will decode, or cannot be made to fit."""


@dataclass(frozen=True)
class EncodedImage:
    data: bytes
    media_type: str
    width: int
    height: int


def sniff(data: bytes) -> str | None:
    """``"pdf"``, ``"png"`` or ``"jpeg"`` from the first bytes, or None for anything else.

    The caller's own claim about the type is never asked for, let alone believed.
    """
    if data.startswith(_PDF_MAGIC):
        return "pdf"
    if data.startswith(_PNG_SIGNATURE):
        return "png"
    if data.startswith(_JPEG_SIGNATURE):
        return "jpeg"
    return None


def open_image(data: bytes, *, max_pixels: int = MAX_IMAGE_PIXELS) -> Image.Image:
    """The picture as plain RGB pixels, upright, with no metadata.

    Transparent areas become white (an invoice with a transparent background would
    otherwise turn black). A photo is turned upright from its EXIF orientation. Only
    the first picture of an animated PNG is read.
    """
    kind = sniff(data)
    if kind not in ("png", "jpeg"):
        raise NotAPdf
    try:
        with Image.open(io.BytesIO(data), formats=["PNG" if kind == "png" else "JPEG"]) as picture:
            width, height = picture.size
            if width < 1 or height < 1:
                raise Unreadable
            if width * height > max_pixels:
                raise ImageTooLarge
            picture.load()
            upright = ImageOps.exif_transpose(picture)
            flat = _flatten(upright)
    except InspectError:
        raise
    except (Image.DecompressionBombError, Image.DecompressionBombWarning):
        # Pillow's own guard (a warning above the limit, an error above twice it; a
        # test setup may turn warnings into errors). Either way the picture is too big.
        raise ImageTooLarge from None
    except Exception as error:
        # Pillow raises many kinds of error on a damaged picture, and some of the
        # messages quote the file. Only the kind of error is logged.
        log.debug("a picture could not be read (%s)", type(error).__name__)
        raise Unreadable from None
    # A new object holds the pixels only; the source's info (EXIF, profile, text) stays behind.
    flat.info = {}
    return flat


def _flatten(picture: Image.Image) -> Image.Image:
    """RGB pixels, with transparency mixed into white."""
    has_alpha = picture.mode in ("RGBA", "LA", "PA") or "transparency" in picture.info
    if not has_alpha:
        return picture.convert("RGB")
    rgba = picture.convert("RGBA")
    background = Image.new("RGB", rgba.size, _WHITE)
    background.paste(rgba, mask=rgba.getchannel("A"))
    return background


def encode_image(
    image: Image.Image,
    *,
    fmt: str = "png",
    max_edge: int = DEFAULT_MAX_EDGE,
    max_bytes: int = MAX_IMAGE_BYTES,
) -> EncodedImage:
    """PNG (or JPEG) bytes of ``image``, within ``max_edge`` and ``max_bytes``.

    The long side is cut down to ``max_edge`` first (never enlarged). A PNG that is
    over ``max_bytes`` is sent as a JPEG instead, and a JPEG that is still too big is
    made smaller by steps until it fits. If it cannot fit above a very small size the
    picture is refused. The image given is used up: its metadata is cleared.
    """
    picture = image if image.mode == "RGB" else image.convert("RGB")
    picture.info = {}
    picture = _fit_edge(picture, max_edge)
    if fmt == "png":
        data = _encode(picture, "PNG")
        if len(data) <= max_bytes:
            return EncodedImage(data, "image/png", *picture.size)
    while True:
        data = _encode(picture, "JPEG")
        if len(data) <= max_bytes:
            return EncodedImage(data, "image/jpeg", *picture.size)
        # Aim a little under the limit; the cost of a JPEG is roughly its pixel count.
        factor = min(0.9, math.sqrt(max_bytes / len(data)) * 0.9)
        smaller = _scaled(picture, factor)
        if max(smaller.size) < _SMALLEST_LONG_EDGE:
            raise ImageTooLarge
        picture = smaller


def _fit_edge(picture: Image.Image, max_edge: int) -> Image.Image:
    longest = max(picture.size)
    if longest <= max_edge:
        return picture
    return _scaled(picture, max_edge / longest, long_edge=max_edge)


def _scaled(picture: Image.Image, factor: float, *, long_edge: int | None = None) -> Image.Image:
    width, height = picture.size
    if long_edge is not None:
        # Exactly the asked-for edge on the long side, the other side in proportion.
        if width >= height:
            size = (long_edge, max(1, round(height * long_edge / width)))
        else:
            size = (max(1, round(width * long_edge / height)), long_edge)
    else:
        size = (max(1, round(width * factor)), max(1, round(height * factor)))
    smaller = picture.resize(size, Image.Resampling.LANCZOS)
    smaller.info = {}
    return smaller


def _encode(picture: Image.Image, fmt: str) -> bytes:
    out = io.BytesIO()
    if fmt == "PNG":
        picture.save(out, "PNG", compress_level=6)
    else:
        picture.save(out, "JPEG", quality=_JPEG_QUALITY, subsampling=0)
    return out.getvalue()
