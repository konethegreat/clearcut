# Written by Kone Tshivhinda & Claude
"""Test images made at test time: nothing here is a real photo or a real document.

Every marker string is invented. They exist so a test can look for them in the bytes
the worker sends back and prove that metadata did not leave.
"""

from __future__ import annotations

import io
import random
import struct
import zlib

from PIL import Image, ImageCms

EXIF_MARKER = "MARKER-DESCRIPTION-IN-EXIF"
COMMENT_MARKER = b"MARKER-JPEG-COMMENT"
GPS_MARKER = "MARKER-GPS-REF"


def png(
    width: int = 20,
    height: int = 10,
    colour: tuple[int, ...] | int = (255, 255, 255),
    mode: str = "RGB",
    *,
    text: str | None = None,
) -> bytes:
    """A plain image saved as PNG, optionally with a text chunk (a place for metadata)."""
    image = Image.new(mode, (width, height), colour)
    out = io.BytesIO()
    if text is None:
        image.save(out, "PNG")
    else:
        from PIL.PngImagePlugin import PngInfo

        info = PngInfo()
        info.add_text("Comment", text)
        image.save(out, "PNG", pnginfo=info)
    return out.getvalue()


def noise(width: int, height: int, *, amplitude: int = 255, seed: int = 1) -> Image.Image:
    """Random pixels (hard to compress), always the same for the same seed."""
    rng = random.Random(seed)  # noqa: S311 - test data, not security
    raw = bytes(rng.randrange(0, amplitude + 1) for _ in range(width * height * 3))
    return Image.frombytes("RGB", (width, height), raw)


def png_of(image: Image.Image) -> bytes:
    out = io.BytesIO()
    image.save(out, "PNG")
    return out.getvalue()


def jpeg(
    width: int = 20,
    height: int = 10,
    colour: tuple[int, ...] | int = (255, 255, 255),
    mode: str = "RGB",
    *,
    orientation: int | None = None,
    with_metadata: bool = False,
) -> bytes:
    """A JPEG, optionally turned by an EXIF orientation and carrying invented metadata."""
    image = Image.new(mode, (width, height), colour)
    options: dict[str, object] = {"quality": 95}
    exif = Image.Exif()
    if orientation is not None:
        exif[0x0112] = orientation
    if with_metadata:
        exif[0x010E] = EXIF_MARKER
        gps = exif.get_ifd(0x8825)
        gps[1] = GPS_MARKER[:2]
        options["comment"] = COMMENT_MARKER
        options["icc_profile"] = ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes()
    if orientation is not None or with_metadata:
        options["exif"] = exif
    out = io.BytesIO()
    image.save(out, "JPEG", **options)
    return out.getvalue()


def _chunk(kind: bytes, body: bytes) -> bytes:
    return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))


def claims_to_be(width: int, height: int) -> bytes:
    """A PNG that only *says* it is this big: a valid header and no real picture data.

    Reading the header tells the size; a worker that checks the size before it decodes
    refuses this at once, and one that decodes first is working on nonsense.
    """
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + _chunk(b"IHDR", header) + _chunk(b"IDAT", b"\x00" * 16)


def png_chunk_types(data: bytes) -> list[str]:
    """The kinds of chunk in a PNG, in order (so a test can see nothing extra is carried)."""
    assert data.startswith(b"\x89PNG\r\n\x1a\n")
    kinds: list[str] = []
    position = 8
    while position < len(data):
        (length,) = struct.unpack(">I", data[position : position + 4])
        kinds.append(data[position + 4 : position + 8].decode("ascii"))
        position += 12 + length
    return kinds


def size_of(data: bytes) -> tuple[int, int]:
    with Image.open(io.BytesIO(data)) as image:
        return image.size


def pixel(data: bytes, x: int, y: int) -> tuple[int, int, int]:
    with Image.open(io.BytesIO(data)) as image:
        value = image.convert("RGB").getpixel((x, y))
    assert isinstance(value, tuple)
    return value[0], value[1], value[2]
