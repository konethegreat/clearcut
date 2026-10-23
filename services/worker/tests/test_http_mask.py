# Written by Kone Tshivhinda & Claude
"""`POST /v1/mask` over HTTP: who may ask, what it answers, and what it refuses."""

from __future__ import annotations

import base64
import json
import logging
from typing import Any

import pytest
from fastapi.testclient import TestClient

from clearcut_worker.app import create_app
from clearcut_worker.errors import InspectError, NotAPdf, TooSlow, Unreadable
from clearcut_worker.images import EncodedImage, ImageTooLarge

from .conftest import AUTH, settings
from .pdfs import pages_of_text
from .pictures import (
    COMMENT_MARKER,
    EXIF_MARKER,
    claims_to_be,
    jpeg,
    noise,
    pixel,
    png,
    png_of,
    size_of,
)

MASK = "/v1/mask"


def body(image: bytes, *boxes: dict[str, int], **extra: Any) -> bytes:
    request: dict[str, Any] = {
        "image": base64.b64encode(image).decode("ascii"),
        "boxes": list(boxes),
    }
    request.update(extra)
    return json.dumps(request).encode()


def box(x: int, y: int, w: int, h: int) -> dict[str, int]:
    return {"x": x, "y": y, "w": w, "h": h}


@pytest.fixture
def client() -> TestClient:
    return TestClient(create_app(settings()))


def test_health_lists_masking_among_the_things_the_worker_can_do(client: TestClient) -> None:
    assert client.get("/health").json()["capabilities"] == ["inspect", "text", "render", "mask"]


# ---------- authentication ----------


def test_anonymous_callers_get_the_same_401_before_anything_is_read(client: TestClient) -> None:
    response = client.post(MASK, content=b"not even json" * 500)
    assert response.status_code == 401
    assert response.json() == {"error": "authentication failed"}
    assert (
        client.post(MASK, content=b"x", headers={"Authorization": "Bearer nope"}).status_code == 401
    )


# ---------- a successful painting ----------


def test_the_boxes_come_back_painted_in_a_png_with_its_size_in_headers(client: TestClient) -> None:
    response = client.post(
        MASK,
        content=body(png(100, 100, (255, 255, 255)), box(200, 200, 100, 100)),
        headers=AUTH,
    )
    assert response.status_code == 200
    assert response.headers["content-type"] == "image/png"
    assert response.headers["x-image-width"] == "100"
    assert response.headers["x-image-height"] == "100"
    assert response.headers["cache-control"] == "no-store"
    assert pixel(response.content, 25, 25) == (0, 0, 0)
    assert pixel(response.content, 5, 5) == (255, 255, 255)


def test_every_pixel_outside_the_boxes_is_unchanged() -> None:
    original = noise(50, 30)
    app = TestClient(create_app(settings()))
    response = app.post(MASK, content=body(png_of(original), box(0, 0, 500, 500)), headers=AUTH)
    assert pixel(response.content, 40, 25) == original.getpixel((40, 25))
    assert pixel(response.content, 10, 10) == (0, 0, 0)


def test_the_content_type_the_caller_claims_is_never_asked_for(client: TestClient) -> None:
    response = client.post(
        MASK,
        content=body(png(), box(0, 0, 1000, 1000)),
        headers={**AUTH, "Content-Type": "text/plain"},
    )
    assert response.status_code == 200


def test_no_boxes_is_allowed_and_the_metadata_is_dropped_all_the_same(client: TestClient) -> None:
    response = client.post(MASK, content=body(jpeg(60, 40, with_metadata=True)), headers=AUTH)
    assert response.status_code == 200
    assert EXIF_MARKER.encode() not in response.content
    assert COMMENT_MARKER not in response.content


def test_a_jpeg_can_be_asked_for_and_the_longest_side_can_be_set(client: TestClient) -> None:
    response = client.post(
        MASK,
        content=body(png(3000, 1000), box(0, 0, 100, 100)),
        headers=AUTH,
        params={"format": "jpeg", "max_edge": 1500},
    )
    assert response.status_code == 200
    assert response.headers["content-type"] == "image/jpeg"
    assert size_of(response.content) == (1500, 500)


def test_exactly_five_hundred_boxes_are_painted(client: TestClient) -> None:
    response = client.post(
        MASK, content=body(png(40, 40), *[box(0, 0, 100, 100)] * 500), headers=AUTH
    )
    assert response.status_code == 200


# ---------- the request ----------


def with_key(**changes: Any) -> bytes:
    request: dict[str, Any] = {
        "image": base64.b64encode(png()).decode("ascii"),
        "boxes": [box(0, 0, 100, 100)],
    }
    request.update(changes)
    return json.dumps(request).encode()


@pytest.mark.parametrize(
    "payload",
    [
        b"not json at all",
        b"[]",
        b"null",
        b'"image"',
        json.dumps({"boxes": []}).encode(),
        json.dumps({"image": "AAAA"}).encode(),
        with_key(image=""),
        with_key(image="***not base64***"),
        with_key(image="abc"),  # wrong padding
        with_key(image=12),
        with_key(image=None),
        with_key(boxes="none"),
        with_key(boxes=None),
        with_key(boxes={"x": 0}),
        with_key(boxes=[box(0, 0, 100, 100)] * 501),
        with_key(boxes=[{"x": 0, "y": 0, "w": 10}]),
        with_key(boxes=[{"x": 0, "y": 0, "w": 10, "h": 10, "extra": 1}]),
        with_key(boxes=[{"x": 0.5, "y": 0, "w": 10, "h": 10}]),
        with_key(boxes=[{"x": True, "y": 0, "w": 10, "h": 10}]),
        with_key(boxes=[{"x": "0", "y": 0, "w": 10, "h": 10}]),
        with_key(boxes=[{"x": None, "y": 0, "w": 10, "h": 10}]),
        with_key(boxes=[box(-1, 0, 10, 10)]),
        with_key(boxes=[box(0, -1, 10, 10)]),
        with_key(boxes=[box(0, 0, 0, 10)]),
        with_key(boxes=[box(0, 0, 10, 0)]),
        with_key(boxes=[box(995, 0, 10, 10)]),
        with_key(boxes=[box(0, 995, 10, 10)]),
        with_key(boxes=[box(1001, 0, 1, 1)]),
        with_key(boxes=[[0, 0, 10, 10]]),
        with_key(extra="field"),
    ],
)
def test_a_request_that_is_not_what_was_promised_is_a_400_that_says_nothing_else(
    client: TestClient, payload: bytes
) -> None:
    response = client.post(MASK, content=payload, headers=AUTH)
    assert response.status_code == 400
    assert response.json() == {"error": "invalid request"}


@pytest.mark.parametrize(
    "params",
    [
        {"format": "gif"},
        {"format": "PNG"},
        {"max_edge": 499},
        {"max_edge": 2577},
        {"max_edge": "wide"},
    ],
)
def test_a_bad_parameter_is_a_400(client: TestClient, params: dict[str, Any]) -> None:
    response = client.post(MASK, content=body(png()), headers=AUTH, params=params)
    assert response.status_code == 400
    assert response.json() == {"error": "invalid request"}


def test_an_empty_body_is_a_400(client: TestClient) -> None:
    response = client.post(MASK, content=b"", headers=AUTH)
    assert response.status_code == 400
    assert response.json() == {"error": "the file is empty"}


def test_a_request_over_the_size_cap_is_refused_from_its_declared_length() -> None:
    small = TestClient(create_app(settings(max_request_bytes=100)))
    response = small.post(MASK, content=b"x" * 500, headers=AUTH)
    assert response.status_code == 413
    assert response.json() == {"error": "file too large", "max_bytes": 100}


def test_a_picture_over_the_picture_cap_is_refused(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("clearcut_worker.app.MAX_MASK_INPUT_BYTES", 100)
    app = TestClient(create_app(settings()))
    response = app.post(MASK, content=body(png_of(noise(30, 30))), headers=AUTH)
    assert response.status_code == 413
    assert response.json() == {"error": "file too large", "max_bytes": 100}


# ---------- what the worker refuses ----------


def test_other_kinds_of_file_are_not_painted(client: TestClient) -> None:
    for blob in (pages_of_text("a"), b"<html>nope</html>", b"GIF89a...."):
        response = client.post(MASK, content=body(blob), headers=AUTH)
        assert response.status_code == 415
        assert response.json() == {"error": "unsupported file type"}


def test_a_damaged_picture_is_a_422_that_reveals_nothing(client: TestClient) -> None:
    response = client.post(MASK, content=body(png_of(noise(40, 40))[:3000]), headers=AUTH)
    assert response.status_code == 422
    assert response.json() == {"error": "the document could not be read"}


def test_a_huge_picture_is_a_422(client: TestClient) -> None:
    response = client.post(MASK, content=body(claims_to_be(30_000, 30_000)), headers=AUTH)
    assert response.status_code == 422
    assert response.json() == {"error": "the image is too large"}


# ---------- how the app uses the masker ----------


def canned() -> Any:
    def masker(*args: Any, **kwargs: Any) -> EncodedImage:
        masker.calls.append((args, kwargs))  # type: ignore[attr-defined]
        return EncodedImage(png(7, 5), "image/png", 7, 5)

    masker.calls = []  # type: ignore[attr-defined]
    return masker


def test_the_app_hands_the_masker_the_decoded_picture_the_boxes_and_the_settings() -> None:
    masker = canned()
    app = create_app(settings(timeout_seconds=12.5), masker=masker)
    picture = png(30, 20)
    response = TestClient(app).post(
        MASK,
        content=body(picture, box(1, 2, 3, 4), box(10, 20, 30, 40)),
        headers=AUTH,
        params={"format": "jpeg", "max_edge": 1500},
    )
    assert response.status_code == 200
    assert response.headers["x-image-width"] == "7"
    ((args, kwargs),) = masker.calls
    assert args == (picture, [(1, 2, 3, 4), (10, 20, 30, 40)])
    assert kwargs == {"fmt": "jpeg", "max_edge": 1500, "timeout_seconds": 12.5}


def test_without_parameters_the_masker_gets_a_png_at_the_default_edge() -> None:
    masker = canned()
    TestClient(create_app(settings(), masker=masker)).post(MASK, content=body(png()), headers=AUTH)
    ((_, kwargs),) = masker.calls
    assert (kwargs["fmt"], kwargs["max_edge"]) == ("png", 2000)


@pytest.mark.parametrize(
    ("error", "status", "body_"),
    [
        (NotAPdf(), 415, {"error": "unsupported file type"}),
        (Unreadable(), 422, {"error": "the document could not be read"}),
        (ImageTooLarge(), 422, {"error": "the image is too large"}),
        (TooSlow(), 504, {"error": "the document took too long to inspect"}),
    ],
)
def test_whatever_the_masker_refuses_with_is_translated(
    error: InspectError, status: int, body_: dict[str, Any]
) -> None:
    def refuse(*args: Any, **kwargs: Any) -> EncodedImage:
        raise error

    response = TestClient(create_app(settings(), masker=refuse)).post(
        MASK, content=body(png()), headers=AUTH
    )
    assert (response.status_code, response.json()) == (status, body_)


def test_logs_carry_sizes_and_outcomes_but_never_the_picture_or_the_token(
    caplog: pytest.LogCaptureFixture, client: TestClient
) -> None:
    with caplog.at_level(logging.DEBUG, logger="clearcut_worker"):
        client.post(MASK, content=body(png(10, 10, text="SECRET-NOTE")), headers=AUTH)
    assert "SECRET" not in caplog.text
    assert "tttttttt" not in caplog.text
    assert "/v1/mask" in caplog.text


def test_a_picture_of_exactly_the_cap_is_allowed_and_one_byte_more_is_not(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr("clearcut_worker.app.MAX_MASK_INPUT_BYTES", 100)
    masker = canned()
    app = TestClient(create_app(settings(), masker=masker))
    assert app.post(MASK, content=body(b"x" * 100), headers=AUTH).status_code == 200
    over = app.post(MASK, content=body(b"x" * 101), headers=AUTH)
    assert over.status_code == 413
    assert over.json() == {"error": "file too large", "max_bytes": 100}
    assert len(masker.calls) == 1


def test_base64_with_anything_but_the_alphabet_in_it_is_refused_not_cleaned_up(
    client: TestClient,
) -> None:
    good = base64.b64encode(png()).decode("ascii")
    for damaged in (
        good[:10] + "\n" + good[10:],
        good[:10] + " " + good[10:],
        good[:10] + "!!" + good[10:],
    ):
        payload = json.dumps({"image": damaged, "boxes": []}).encode()
        response = client.post(MASK, content=payload, headers=AUTH)
        assert response.status_code == 400
        assert response.json() == {"error": "invalid request"}
