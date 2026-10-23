# Written by Kone Tshivhinda & Claude
"""Masking in a child process, and not believing what the child says (ADR-0016)."""

from __future__ import annotations

import time
from typing import Any

import pytest

from clearcut_worker.errors import NotAPdf, TooSlow, Unreadable
from clearcut_worker.images import EncodedImage, ImageTooLarge
from clearcut_worker.isolation import mask_isolated
from clearcut_worker.mask import mask_image

from . import children
from .pictures import claims_to_be, jpeg, noise, pixel, png, png_of
from .test_isolation import no_children_left

FAR = time.monotonic() + 3600


def mask(data: bytes, boxes: list[tuple[int, int, int, int]], **options: Any) -> EncodedImage:
    settings: dict[str, Any] = {"fmt": "png", "max_edge": 2000, "timeout_seconds": 30.0}
    settings.update(options)
    return mask_isolated(data, boxes, **settings)


# ---------- the whole trip ----------


def test_the_answer_from_the_child_equals_masking_in_this_process() -> None:
    source = png_of(noise(60, 40))
    boxes = [(100, 200, 300, 400), (700, 700, 200, 200)]
    inside = mask_image(source, boxes, deadline=FAR)
    assert mask(source, boxes) == inside
    assert no_children_left()


def test_the_boxes_are_painted_in_the_picture_that_comes_back() -> None:
    result = mask(png(100, 100, (255, 255, 255)), [(200, 200, 100, 100)])
    assert pixel(result.data, 25, 25) == (0, 0, 0)
    assert pixel(result.data, 5, 5) == (255, 255, 255)


def test_a_jpeg_in_and_a_jpeg_out_make_the_trip_too() -> None:
    result = mask(jpeg(100, 50, orientation=6), [(0, 0, 500, 500)], fmt="jpeg")
    assert result.media_type == "image/jpeg"
    assert (result.width, result.height) == (50, 100)


def test_every_kind_of_refusal_survives_the_trip_back_from_the_child() -> None:
    with pytest.raises(NotAPdf):
        mask(b"%PDF-1.4\n...", [])
    with pytest.raises(NotAPdf):
        mask(b"<html>nope</html>", [])
    with pytest.raises(Unreadable):
        mask(png_of(noise(40, 40))[:3000], [])
    with pytest.raises(ImageTooLarge):
        mask(claims_to_be(30_000, 30_000), [])
    with pytest.raises(TooSlow):
        mask(png(), [], timeout_seconds=0.0)


def test_a_box_that_cannot_be_right_is_an_unreadable_request_not_a_crash() -> None:
    # The route checks boxes first; if one reached the child it still ends in a named refusal.
    with pytest.raises(Unreadable):
        mask(png(), [(0, 0, 0, 0)])


# ---------- hostile children ----------


def test_a_masker_stuck_for_ever_is_killed_at_the_hard_limit() -> None:
    started = time.monotonic()
    with pytest.raises(TooSlow):
        mask(png(), [], timeout_seconds=0.5, grace_seconds=0.5, target=children.mask_hang)
    assert time.monotonic() - started < 15
    assert no_children_left()


def test_a_masker_that_dies_hard_is_an_unreadable_picture() -> None:
    with pytest.raises(Unreadable):
        mask(png(), [], target=children.mask_crash)
    assert no_children_left()


def test_an_answer_that_is_not_a_picture_of_the_size_it_claims_is_unreadable() -> None:
    with pytest.raises(Unreadable):
        mask(b"nonsense", [], target=children.mask_echo)
    with pytest.raises(Unreadable):
        mask(b"x" * 2048, [], target=children.mask_echo, max_answer_bytes=1024)


def test_the_child_brings_a_big_picture_down_to_the_edge_asked_for() -> None:
    result = mask(png(3000, 1000), [], max_edge=2000)
    assert (result.width, result.height) == (2000, 667)


def frame(width: int, height: int, image: bytes) -> bytes:
    import struct

    return b"CCIMG1" + b"P" + struct.pack(">II", width, height) + image


def test_the_parent_refuses_an_answer_bigger_than_the_edge_it_asked_for() -> None:
    from .test_render_isolation import a_png

    assert mask(frame(2000, 20, a_png(2000, 20)), [], target=children.mask_echo).width == 2000
    with pytest.raises(Unreadable):
        mask(frame(2001, 20, a_png(2001, 20)), [], target=children.mask_echo)


def test_the_answer_cap_defaults_to_the_one_for_pictures() -> None:
    import inspect

    from clearcut_worker.isolation import DEFAULT_RENDER_ANSWER_BYTES

    default = inspect.signature(mask_isolated).parameters["max_answer_bytes"].default
    assert default == DEFAULT_RENDER_ANSWER_BYTES
