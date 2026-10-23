# Written by Kone Tshivhinda & Claude
"""Painting boxes black on a picture, and nothing else (ADR-0016): the worker has no rules."""

from __future__ import annotations

import io
import time

import pytest
from PIL import Image

from clearcut_worker.errors import NotAPdf, TooSlow, Unreadable
from clearcut_worker.images import ImageTooLarge, open_image
from clearcut_worker.mask import MAX_BOXES, mask_image, pixel_rectangle

from .pictures import (
    COMMENT_MARKER,
    EXIF_MARKER,
    claims_to_be,
    jpeg,
    noise,
    png,
    png_of,
    size_of,
)

FAR = time.monotonic() + 3600
BLACK = (0, 0, 0)


def masked(data: bytes, *boxes: tuple[int, int, int, int], **options: object) -> Image.Image:
    result = mask_image(data, list(boxes), deadline=FAR, **options)  # type: ignore[arg-type]
    return Image.open(io.BytesIO(result.data)).convert("RGB")


# ---------- from thousandths to pixels ----------


def test_a_box_in_thousandths_is_the_same_share_of_the_picture_in_pixels() -> None:
    assert pixel_rectangle((100, 200, 200, 400), (100, 50)) == (10, 10, 30, 30)
    assert pixel_rectangle((0, 0, 1000, 1000), (100, 50)) == (0, 0, 100, 50)


def test_the_rectangle_is_rounded_outwards_so_it_never_covers_less_than_asked() -> None:
    # 300 to 400 thousandths of 7 pixels is 2.1 to 2.8: pixels 2 and up to 3.
    assert pixel_rectangle((300, 300, 100, 100), (7, 7)) == (2, 2, 3, 3)
    # 1 to 2 thousandths of a 100 pixel picture is a tenth of a pixel, and still one pixel.
    assert pixel_rectangle((1, 1, 1, 1), (100, 100)) == (0, 0, 1, 1)


def test_a_box_that_ends_at_the_edge_ends_at_the_last_pixel() -> None:
    assert pixel_rectangle((900, 900, 100, 100), (333, 777)) == (299, 699, 333, 777)


def test_every_valid_box_covers_at_least_one_pixel_whatever_the_size() -> None:
    for width in (1, 2, 7, 333, 1000, 1001, 4000):
        for x in (0, 1, 499, 500, 998, 999):
            x0, _y0, x1, _y1 = pixel_rectangle((x, 0, 1, 1), (width, width))
            assert 0 <= x0 < x1 <= width


# ---------- painting ----------


def test_the_pixels_in_the_box_are_black_and_every_other_pixel_is_unchanged() -> None:
    original = noise(60, 40)
    result = masked(png_of(original), (100, 200, 300, 400))  # pixels 6..24 across, 8..24 down
    x0, y0, x1, y1 = pixel_rectangle((100, 200, 300, 400), (60, 40))
    for x in range(60):
        for y in range(40):
            inside = x0 <= x < x1 and y0 <= y < y1
            assert result.getpixel((x, y)) == (BLACK if inside else original.getpixel((x, y)))


def test_the_edges_of_the_box_are_exact() -> None:
    original = Image.new("RGB", (100, 100), (200, 100, 50))
    result = masked(png_of(original), (200, 200, 100, 100))  # pixels 20 up to 30
    assert result.getpixel((20, 20)) == BLACK
    assert result.getpixel((29, 29)) == BLACK
    assert result.getpixel((19, 25)) == (200, 100, 50)
    assert result.getpixel((30, 25)) == (200, 100, 50)
    assert result.getpixel((25, 19)) == (200, 100, 50)
    assert result.getpixel((25, 30)) == (200, 100, 50)


def test_several_boxes_overlapping_or_not_are_all_painted() -> None:
    original = Image.new("RGB", (100, 100), (255, 255, 255))
    result = masked(png_of(original), (0, 0, 100, 100), (550, 550, 100, 100), (555, 555, 20, 20))
    assert result.getpixel((5, 5)) == BLACK
    assert result.getpixel((55, 55)) == BLACK
    assert result.getpixel((64, 64)) == BLACK
    assert result.getpixel((70, 70)) == (255, 255, 255)
    assert result.getpixel((20, 20)) == (255, 255, 255)


def test_no_boxes_gives_back_the_same_pixels() -> None:
    original = noise(30, 20)
    assert masked(png_of(original)).tobytes() == original.tobytes()


def test_five_hundred_boxes_are_allowed_and_five_hundred_and_one_are_not() -> None:
    assert MAX_BOXES == 500
    boxes = [(0, 0, 10, 10)] * 500
    assert masked(png(20, 20), *boxes).getpixel((0, 0)) == BLACK
    with pytest.raises(ValueError, match="500"):
        mask_image(png(20, 20), [(0, 0, 10, 10)] * 501, deadline=FAR)


@pytest.mark.parametrize(
    "box",
    [
        (0, 0, 0, 10),  # no width
        (0, 0, 10, 0),  # no height
        (-1, 0, 10, 10),
        (0, -1, 10, 10),
        (995, 0, 10, 10),  # leaves the picture
        (0, 995, 10, 10),
        (1001, 0, 1, 1),
        (0, 0, 1001, 1),
        (0.5, 0, 10, 10),
        (True, 0, 10, 10),
        (0, 0, 10),
        (0, 0, 10, 10, 10),
        "box",
    ],
)
def test_a_box_that_cannot_be_right_is_refused(box: object) -> None:
    with pytest.raises(ValueError, match="box"):
        mask_image(png(20, 20), [box], deadline=FAR)


# ---------- what comes back carries only pixels ----------


def test_nothing_is_left_of_what_was_under_the_box() -> None:
    secret = Image.new("RGB", (50, 50), (1, 2, 3))
    for x in range(50):
        for y in range(50):
            secret.putpixel((x, y), (x * 5 % 256, y * 5 % 256, 77))
    result = masked(png_of(secret), (0, 0, 1000, 1000))
    assert result.getextrema() == ((0, 0), (0, 0), (0, 0))


def test_transparency_cannot_hide_what_was_painted_or_leave_an_alpha_channel() -> None:
    translucent = Image.new("RGBA", (40, 40), (10, 20, 30, 0))
    out = io.BytesIO()
    translucent.save(out, "PNG")
    result = mask_image(out.getvalue(), [(0, 0, 500, 500)], deadline=FAR)
    with Image.open(io.BytesIO(result.data)) as again:
        assert again.mode == "RGB"
        assert again.getpixel((5, 5)) == BLACK
        assert again.getpixel((30, 30)) == (255, 255, 255)


@pytest.mark.parametrize("fmt", ["png", "jpeg"])
def test_no_metadata_survives_masking(fmt: str) -> None:
    source = jpeg(60, 40, with_metadata=True)
    data = mask_image(source, [(0, 0, 500, 500)], deadline=FAR, fmt=fmt).data
    assert EXIF_MARKER.encode() not in data
    assert COMMENT_MARKER not in data
    assert b"Exif" not in data
    assert b"acsp" not in data


def test_a_text_chunk_in_a_png_does_not_survive_either() -> None:
    data = mask_image(png(20, 10, text="SECRET-NOTE"), [], deadline=FAR).data
    assert b"SECRET-NOTE" not in data


def test_the_boxes_apply_to_the_picture_as_a_reader_sees_it_after_turning_it_upright() -> None:
    photo = jpeg(200, 100, orientation=6)
    upright = open_image(photo)
    assert upright.size == (100, 200)
    result = masked(photo, (0, 0, 500, 500))
    assert result.size == (100, 200)
    assert result.getpixel((10, 10)) == BLACK
    assert result.getpixel((90, 190)) == upright.getpixel((90, 190))


def test_a_jpeg_can_be_the_result_and_the_box_is_still_black_in_it() -> None:
    result = mask_image(png(60, 40), [(0, 0, 1000, 1000)], deadline=FAR, fmt="jpeg")
    assert result.media_type == "image/jpeg"
    with Image.open(io.BytesIO(result.data)) as again:
        middle = again.convert("RGB").getpixel((30, 20))
    assert isinstance(middle, tuple)
    assert max(middle) < 20


def test_a_picture_over_the_edge_limit_is_painted_and_then_brought_down() -> None:
    result = mask_image(
        png(3000, 1000, (255, 255, 255)), [(0, 0, 500, 1000)], deadline=FAR, max_edge=2000
    )
    assert (result.width, result.height) == (2000, 667)
    assert size_of(result.data) == (2000, 667)
    with Image.open(io.BytesIO(result.data)) as again:
        assert again.convert("RGB").getpixel((500, 300)) == BLACK
        assert again.convert("RGB").getpixel((1500, 300)) == (255, 255, 255)


def test_the_result_is_a_png_by_default_and_says_how_big_it_is() -> None:
    result = mask_image(png(30, 20), [(0, 0, 100, 100)], deadline=FAR)
    assert result.media_type == "image/png"
    assert (result.width, result.height) == (30, 20)


# ---------- what is refused ----------


def test_only_png_and_jpeg_are_painted() -> None:
    for blob in (b"%PDF-1.4\n...", b"<html>nope</html>", b"GIF89a....", b""):
        with pytest.raises(NotAPdf):
            mask_image(blob, [], deadline=FAR)


def test_a_damaged_picture_is_unreadable_and_a_huge_one_is_refused() -> None:
    with pytest.raises(Unreadable):
        mask_image(png_of(noise(40, 40))[:3000], [], deadline=FAR)
    with pytest.raises(ImageTooLarge):
        mask_image(claims_to_be(30_000, 30_000), [], deadline=FAR)


def test_the_pixel_limit_can_be_set() -> None:
    assert mask_image(png(10, 10), [], deadline=FAR, max_pixels=100)
    with pytest.raises(ImageTooLarge):
        mask_image(png(11, 10), [], deadline=FAR, max_pixels=100)


def test_a_deadline_that_has_passed_stops_it_before_the_picture_is_opened() -> None:
    with pytest.raises(TooSlow):
        mask_image(png(), [], deadline=time.monotonic() - 1)


def test_the_messages_never_contain_picture_content() -> None:
    with pytest.raises(Unreadable) as caught:
        mask_image(png(10, 10, text="SECRET-NOTE")[:-30], [], deadline=FAR)
    assert "SECRET" not in str(caught.value)


def test_a_box_given_as_a_list_is_as_good_as_one_given_as_a_tuple() -> None:
    result = mask_image(png(100, 100), [[0, 0, 500, 500]], deadline=FAR)
    with Image.open(io.BytesIO(result.data)) as again:
        assert again.convert("RGB").getpixel((10, 10)) == BLACK


def test_the_byte_limit_is_passed_on_so_a_big_result_is_made_smaller() -> None:
    result = mask_image(png_of(noise(400, 400)), [(0, 0, 100, 100)], deadline=FAR, max_bytes=60_000)
    assert len(result.data) <= 60_000
    assert result.width < 400


def test_the_picture_is_not_even_opened_when_the_time_is_already_up(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def must_not_open(*args: object, **kwargs: object) -> None:
        raise AssertionError("opened a picture after the deadline")

    monkeypatch.setattr("clearcut_worker.mask.open_image", must_not_open)
    with pytest.raises(TooSlow):
        mask_image(png(), [], deadline=time.monotonic() - 1)


def test_a_picture_that_takes_too_long_to_open_is_too_slow_and_is_not_painted(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    real = open_image

    def slow_open(*args: object, **kwargs: object) -> Image.Image:
        time.sleep(0.3)
        return real(*args, **kwargs)  # type: ignore[arg-type]

    def must_not_paint(*args: object, **kwargs: object) -> None:
        raise AssertionError("painted after the deadline")

    monkeypatch.setattr("clearcut_worker.mask.open_image", slow_open)
    monkeypatch.setattr("clearcut_worker.mask.paint", must_not_paint)
    with pytest.raises(TooSlow):
        mask_image(png(), [(0, 0, 100, 100)], deadline=time.monotonic() + 0.15)
