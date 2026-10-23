# Written by Kone Tshivhinda & Claude
"""Drawing a page in a child process, and not believing what the child says (ADR-0016)."""

from __future__ import annotations

import struct
import time
import zlib
from typing import Any

import pytest

from clearcut_worker.images import MAX_IMAGE_BYTES, EncodedImage, ImageTooLarge
from clearcut_worker.inspect import Encrypted, NotAPdf, TooManyPages, TooSlow, Unreadable
from clearcut_worker.isolation import (
    DEFAULT_RENDER_ANSWER_BYTES,
    _jpeg_size,
    decode_render_answer,
    render_isolated,
)
from clearcut_worker.pagetext import PageOutOfRange
from clearcut_worker.render import PageTooBig, render_page

from . import children
from .conftest import make_encrypted_pdf
from .pdfs import one_page, pages_of_text
from .pictures import claims_to_be, jpeg, noise, png_of
from .test_isolation import no_children_left

FAR = time.monotonic() + 3600
SQUARE = "0 0 0 rg 50 50 100 100 re f\n"


def render(data: bytes, page: int = 1, **options: Any) -> EncodedImage:
    settings: dict[str, Any] = {
        "dpi": 144,
        "fmt": "png",
        "max_edge": 2000,
        "max_pages": 50,
        "timeout_seconds": 30.0,
    }
    settings.update(options)
    return render_isolated(data, page, **settings)


# ---------- the whole trip ----------


def test_the_answer_from_the_child_equals_drawing_in_this_process() -> None:
    pdf = one_page(SQUARE)
    inside = render_page(pdf, 1, dpi=144, max_pages=50, deadline=FAR)
    assert render(pdf) == inside
    assert no_children_left()


def test_a_jpeg_can_be_asked_for_and_comes_back_as_one() -> None:
    result = render(one_page(SQUARE), fmt="jpeg")
    assert result.media_type == "image/jpeg"
    assert result.data.startswith(b"\xff\xd8\xff")


def test_a_picture_document_makes_the_trip_too() -> None:
    result = render(png_of(noise(40, 30)))
    assert (result.width, result.height) == (40, 30)
    assert render(jpeg(100, 50, orientation=6)).width == 50


def test_every_kind_of_refusal_survives_the_trip_back_from_the_child() -> None:
    with pytest.raises(NotAPdf):
        render(b"<html>nope</html>")
    with pytest.raises(Encrypted):
        render(make_encrypted_pdf())
    with pytest.raises(Unreadable):
        render(b"%PDF-1.4\nSECRET garbage")
    with pytest.raises(TooManyPages) as many:
        render(pages_of_text("a", "b", "c"), max_pages=2)
    assert many.value.max_pages == 2
    with pytest.raises(PageOutOfRange) as out:
        render(pages_of_text("a", "b"), 3)
    assert out.value.pages == 2
    with pytest.raises(PageTooBig):
        render(one_page("", media_box="[0 0 14401 100]"))
    with pytest.raises(ImageTooLarge):
        render(claims_to_be(30_000, 30_000))
    with pytest.raises(TooSlow):
        render(pages_of_text("a"), timeout_seconds=0.0)


# ---------- hostile children ----------


def test_a_renderer_stuck_for_ever_is_killed_at_the_hard_limit() -> None:
    started = time.monotonic()
    with pytest.raises(TooSlow):
        render(
            pages_of_text("a"),
            timeout_seconds=0.5,
            grace_seconds=0.5,
            target=children.render_hang,
        )
    assert time.monotonic() - started < 15
    assert no_children_left()


def test_a_renderer_that_dies_hard_is_an_unreadable_document() -> None:
    with pytest.raises(Unreadable):
        render(pages_of_text("a"), target=children.render_crash)
    assert no_children_left()


def echo(answer: bytes, **options: Any) -> EncodedImage:
    return render(answer, target=children.render_echo, **options)


def test_an_answer_over_the_size_cap_is_unreadable() -> None:
    with pytest.raises(Unreadable):
        echo(b"x" * 2048, max_answer_bytes=1024)
    assert DEFAULT_RENDER_ANSWER_BYTES > MAX_IMAGE_BYTES
    assert DEFAULT_RENDER_ANSWER_BYTES < MAX_IMAGE_BYTES + 4096


# ---------- checking an answer field by field ----------

PNG_END = b"\x00\x00\x00\x00IEND\xaeB`\x82"


def a_png(width: int, height: int, extra: bytes = b"") -> bytes:
    """Just enough of a PNG to pass the checks the parent makes (it is never decoded)."""
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    crc = struct.pack(">I", zlib.crc32(b"IHDR" + header))
    ihdr = struct.pack(">I", 13) + b"IHDR" + header + crc
    return b"\x89PNG\r\n\x1a\n" + ihdr + extra + PNG_END


def a_jpeg(width: int = 30, height: int = 20, *, before: bytes = b"") -> bytes:
    """Just enough of a JPEG to pass the checks the parent makes (it is never decoded)."""
    app0 = b"\xff\xe0" + struct.pack(">H", 16) + b"JFIF\x00" + b"\x00" * 9
    frame_header = (
        struct.pack(">BHHB", 8, height, width, 3) + b"\x01\x11\x00\x02\x11\x01\x03\x11\x01"
    )
    sof = b"\xff\xc0" + struct.pack(">H", 2 + len(frame_header)) + frame_header
    return b"\xff\xd8" + app0 + before + sof + b"\xff\xda\x00\x02" + b"\x00" * 4 + b"\xff\xd9"


def frame(kind: bytes, width: int, height: int, image: bytes) -> bytes:
    return b"CCIMG1" + kind + struct.pack(">II", width, height) + image


def decode(raw: bytes, **options: Any) -> EncodedImage:
    settings: dict[str, Any] = {"fmt": "png", "max_edge": 2000, "max_bytes": 5000, "max_pages": 9}
    settings.update(options)
    return decode_render_answer(raw, **settings)


def test_a_well_formed_png_answer_is_accepted() -> None:
    data = a_png(30, 20)
    assert decode(frame(b"P", 30, 20, data)) == EncodedImage(data, "image/png", 30, 20)


def test_a_jpeg_answer_is_accepted_and_is_allowed_when_a_png_was_asked_for() -> None:
    data = a_jpeg()
    expected = EncodedImage(data, "image/jpeg", 30, 20)
    assert decode(frame(b"J", 30, 20, data)) == expected
    assert decode(frame(b"J", 30, 20, data), fmt="jpeg") == expected


def test_a_png_is_not_accepted_when_a_jpeg_was_asked_for() -> None:
    with pytest.raises(Unreadable):
        decode(frame(b"P", 30, 20, a_png(30, 20)), fmt="jpeg")


@pytest.mark.parametrize(
    "answer",
    [
        b"",
        b"CCIMG1",
        b"CCIMG1P",
        b"CCIMG1P" + struct.pack(">II", 30, 20),  # no image at all
        b"CCIMG2P" + struct.pack(">II", 30, 20) + b"x",
        b"nonsense",
        b'["ok", 1, 1]',
        frame(b"X", 30, 20, a_png(30, 20)),  # an unknown kind
        frame(b"P", 0, 20, a_png(0, 20)),
        frame(b"P", 30, 0, a_png(30, 0)),
        frame(b"P", 2001, 20, a_png(2001, 20)),  # over the edge asked for
        frame(b"P", 30, 2001, a_png(30, 2001)),
        frame(b"P", 30, 20, a_png(31, 20)),  # the picture says it is another size
        frame(b"P", 30, 20, a_png(30, 21)),
        frame(b"P", 30, 20, a_jpeg()),  # not what it claims to be
        frame(b"J", 30, 20, a_png(30, 20)),
        frame(b"P", 30, 20, a_png(30, 20)[:-5]),  # the end is missing
        frame(b"J", 30, 20, a_jpeg()[:-2]),
        frame(b"P", 30, 20, a_png(30, 20, b"x" * 6000)),  # over the byte limit
        frame(b"P", 30, 20, b""),
    ],
)
def test_an_answer_that_is_not_what_was_promised_is_unreadable(answer: bytes) -> None:
    with pytest.raises(Unreadable):
        decode(answer)


def test_the_byte_limit_is_exact() -> None:
    padding = 5000 - len(a_png(30, 20))
    assert decode(frame(b"P", 30, 20, a_png(30, 20, b"x" * padding)))
    with pytest.raises(Unreadable):
        decode(frame(b"P", 30, 20, a_png(30, 20, b"x" * (padding + 1))))


def test_the_edge_limit_is_exact() -> None:
    assert decode(frame(b"P", 2000, 20, a_png(2000, 20))).width == 2000


def test_the_error_names_come_back_as_themselves() -> None:
    for name, expected in [
        ("NotAPdf", NotAPdf),
        ("Encrypted", Encrypted),
        ("Unreadable", Unreadable),
        ("TooSlow", TooSlow),
        ("PageTooBig", PageTooBig),
        ("ImageTooLarge", ImageTooLarge),
        ("TooManyPages", TooManyPages),
    ]:
        with pytest.raises(expected):
            decode(b"CCERR1" + name.encode())
    with pytest.raises(TooManyPages) as many:
        decode(b"CCERR1TooManyPages")
    assert many.value.max_pages == 9
    with pytest.raises(PageOutOfRange) as out:
        decode(b"CCERR1PageOutOfRange:7")
    assert out.value.pages == 7


@pytest.mark.parametrize(
    "answer",
    [
        b"CCERR1",
        b"CCERR1Surprise",
        b"CCERR1PageOutOfRange",
        b"CCERR1PageOutOfRange:",
        b"CCERR1PageOutOfRange:-1",
        b"CCERR1PageOutOfRange:1.5",
        b"CCERR1PageOutOfRange:12345678901",
        b"CCERR1PageOutOfRange:7 and more",
        b"CCERR1TooSlow and more",
        b"CCERR1NotAPdf\xff",
        "CCERR1Encryptedé".encode(),
        b"CCERR1TooSlow:5",
    ],
)
def test_an_error_that_is_not_one_of_ours_is_unreadable_not_believed(answer: bytes) -> None:
    with pytest.raises(Unreadable):
        decode(answer)


def test_a_real_jpeg_from_the_encoder_passes_the_parents_checks() -> None:
    from clearcut_worker.images import encode_image

    data = encode_image(noise(37, 23), fmt="jpeg").data
    assert decode(frame(b"J", 37, 23, data), fmt="jpeg").data == data


def test_a_jpeg_must_state_the_size_the_child_claimed() -> None:
    assert decode(frame(b"J", 30, 20, a_jpeg(30, 20)))
    for width, height in [(31, 20), (30, 21), (20, 30)]:
        with pytest.raises(Unreadable):
            decode(frame(b"J", 30, 20, a_jpeg(width, height)))


def test_a_jpeg_may_have_other_segments_and_fill_bytes_before_its_frame_header() -> None:
    comment = b"\xff\xfe" + struct.pack(">H", 6) + b"abcd"
    assert decode(frame(b"J", 30, 20, a_jpeg(before=comment * 5)))
    assert decode(frame(b"J", 30, 20, a_jpeg(before=b"\xff\xff\xff" + comment)))


@pytest.mark.parametrize(
    "mangle",
    [
        lambda j: j[2:],  # no start marker
        lambda j: j[:30],  # cut off before the frame header (and the end)
        lambda j: j.replace(b"\xff\xc0", b"\xff\xdb", 1),  # no frame header at all
        lambda j: j[:2] + b"\x00" + j[3:],  # a segment that does not start with 0xFF
        lambda j: j[:4] + b"\x00\x01" + j[6:],  # a segment length below 2
        lambda j: j[:2] + b"\xff\xe0\xff\xff" + j[6:],  # a length that runs off the end
    ],
)
def test_a_jpeg_without_a_sound_frame_header_is_unreadable(mangle: Any) -> None:
    with pytest.raises(Unreadable):
        decode(frame(b"J", 30, 20, mangle(a_jpeg())))


def test_a_jpeg_with_too_many_segments_before_its_frame_header_is_unreadable() -> None:
    filler = b"\xff\xfe\x00\x02" * 1100
    with pytest.raises(Unreadable):
        decode(frame(b"J", 30, 20, a_jpeg(before=filler)))


def test_a_long_answer_that_is_not_a_frame_is_unreadable() -> None:
    with pytest.raises(Unreadable):
        decode(b"X" * 40)


def test_a_png_needs_its_signature_and_its_header_name() -> None:
    good = a_png(30, 20)
    assert decode(frame(b"P", 30, 20, good))
    with pytest.raises(Unreadable):
        decode(frame(b"P", 30, 20, b"\x00" * 8 + good[8:]))
    with pytest.raises(Unreadable):
        decode(frame(b"P", 30, 20, good[:12] + b"IDAT" + good[16:]))


def a_frame_header(width: int, height: int) -> bytes:
    body = struct.pack(">BHHB", 8, height, width, 3) + b"\x01\x11\x00\x02\x11\x01\x03\x11\x01"
    return b"\xff\xc0" + struct.pack(">H", 2 + len(body)) + body


def test_an_answer_with_a_wrong_frame_name_is_unreadable_even_if_the_rest_is_perfect() -> None:
    perfect = frame(b"P", 30, 20, a_png(30, 20))
    assert decode(perfect)
    with pytest.raises(Unreadable):
        decode(b"CCIMG2" + perfect[6:])


def test_a_byte_that_is_not_a_marker_stops_the_walk_even_if_a_marker_follows() -> None:
    # 0x00 where a marker should start, then bytes that would read as a frame header.
    disguised = b"\x00" + a_frame_header(30, 20)[1:]
    assert _jpeg_size(a_jpeg(before=disguised)) is None
    with pytest.raises(Unreadable):
        decode(frame(b"J", 30, 20, a_jpeg(before=disguised)))


def test_markers_that_carry_no_length_are_stepped_over() -> None:
    restarts = b"\xff\xd0\xff\xd1\xff\x01"
    assert _jpeg_size(a_jpeg(before=restarts)) == (30, 20)
    assert decode(frame(b"J", 30, 20, a_jpeg(before=restarts)))


def test_the_start_of_scan_or_the_end_of_the_picture_before_a_frame_header_stops_the_walk() -> None:
    for marker in (b"\xff\xda", b"\xff\xd9"):
        assert _jpeg_size(a_jpeg(before=marker + b"\x00\x02")) is None


def test_a_frame_header_too_short_to_hold_a_size_is_not_believed() -> None:
    short = b"\xff\xc0\x00\x04\x08\x00\x14\x00\x1e\x03"
    fake = a_jpeg(before=b"")
    cut = fake.replace(a_frame_header(30, 20), short, 1)
    assert _jpeg_size(cut) is None


def test_a_frame_header_cut_off_before_its_size_is_not_believed_and_does_not_crash() -> None:
    assert _jpeg_size(b"\xff\xd8\xff\xc0\x00\x0b\x08") is None
    assert _jpeg_size(b"\xff\xd8\xff\xc0\x00\x0b\x08\x00\x14\x00") is None
    assert _jpeg_size(b"\xff\xd8\xff\xc0\x00\x0b\x08\x00\x14\x00\x1e") == (30, 20)


def test_a_jpeg_must_open_with_the_start_of_image_marker() -> None:
    good = a_jpeg()
    assert decode(frame(b"J", 30, 20, good))
    with pytest.raises(Unreadable):
        decode(frame(b"J", 30, 20, b"\x00\x00" + good[2:]))
