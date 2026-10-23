# Written by Kone Tshivhinda & Claude
"""`POST /v1/render` over HTTP: who may ask, what it answers, and what it refuses."""

from __future__ import annotations

import logging
from typing import Any

import pytest
from fastapi.testclient import TestClient

from clearcut_worker.app import create_app
from clearcut_worker.images import EncodedImage, ImageTooLarge
from clearcut_worker.inspect import (
    Encrypted,
    InspectError,
    NotAPdf,
    TooManyPages,
    TooSlow,
    Unreadable,
)
from clearcut_worker.pagetext import PageOutOfRange
from clearcut_worker.render import PageTooBig

from .conftest import AUTH, make_encrypted_pdf, settings
from .pdfs import PageSpec, PdfBuilder, one_page, pages_of_text
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

RENDER = "/v1/render"
SQUARE = "0 0 0 rg 50 50 100 100 re f\n"


@pytest.fixture
def client() -> TestClient:
    return TestClient(create_app(settings()))


def test_health_lists_rendering_among_the_things_the_worker_can_do(client: TestClient) -> None:
    assert client.get("/health").json()["capabilities"] == ["inspect", "text", "render"]


# ---------- authentication ----------


def test_anonymous_callers_get_the_same_401_before_anything_is_read(client: TestClient) -> None:
    response = client.post(RENDER, content=b"x" * 5000, params={"page": "banana"})
    assert response.status_code == 401
    assert response.json() == {"error": "authentication failed"}
    assert (
        client.post(RENDER, content=b"x", headers={"Authorization": "Bearer nope"}).status_code
        == 401
    )


# ---------- a successful drawing ----------


def test_a_page_comes_back_as_a_png_with_its_size_in_headers(client: TestClient) -> None:
    response = client.post(RENDER, content=one_page(SQUARE), headers=AUTH, params={"dpi": 144})
    assert response.status_code == 200
    assert response.headers["content-type"] == "image/png"
    assert response.headers["x-image-width"] == "400"
    assert response.headers["x-image-height"] == "400"
    assert response.headers["cache-control"] == "no-store"
    assert response.content.startswith(b"\x89PNG")
    assert size_of(response.content) == (400, 400)


def test_the_defaults_are_page_one_at_150_dpi_as_a_png(client: TestClient) -> None:
    response = client.post(RENDER, content=one_page(SQUARE), headers=AUTH)
    assert response.status_code == 200
    assert response.headers["content-type"] == "image/png"
    # 200 points at 150 dpi.
    assert abs(int(response.headers["x-image-width"]) - 417) <= 1


def test_a_later_page_and_a_jpeg_can_be_asked_for(client: TestClient) -> None:
    pdf = PdfBuilder(pages=[PageSpec(content=""), PageSpec(content=SQUARE)]).build()
    response = client.post(
        RENDER, content=pdf, headers=AUTH, params={"page": 2, "format": "jpeg", "dpi": 144}
    )
    assert response.status_code == 200
    assert response.headers["content-type"] == "image/jpeg"
    assert pixel(response.content, 200, 200)[0] < 40  # the square, not the blank page


def test_the_longest_side_can_be_raised_for_a_dense_scan(client: TestClient) -> None:
    big = one_page("", media_box="[0 0 595 842]")
    plain = client.post(RENDER, content=big, headers=AUTH, params={"dpi": 300})
    wide = client.post(RENDER, content=big, headers=AUTH, params={"dpi": 300, "max_edge": 2576})
    assert plain.headers["x-image-height"] == "2000"
    assert wide.headers["x-image-height"] == "2576"


def test_a_picture_document_is_rendered_upright_without_its_metadata(client: TestClient) -> None:
    response = client.post(
        RENDER, content=jpeg(200, 100, orientation=6, with_metadata=True), headers=AUTH
    )
    assert response.status_code == 200
    assert (response.headers["x-image-width"], response.headers["x-image-height"]) == ("100", "200")
    assert EXIF_MARKER.encode() not in response.content
    assert COMMENT_MARKER not in response.content


# ---------- the request ----------


@pytest.mark.parametrize(
    "params",
    [
        {"page": 0},
        {"page": -1},
        {"page": "one"},
        {"page": 1.5},
        {"dpi": 99},
        {"dpi": 301},
        {"dpi": "high"},
        {"dpi": 0},
        {"format": "gif"},
        {"format": "PNG"},
        {"format": ""},
        {"max_edge": 499},
        {"max_edge": 2577},
        {"max_edge": "wide"},
    ],
)
def test_a_bad_parameter_is_a_400_that_says_nothing_else(
    client: TestClient, params: dict[str, Any]
) -> None:
    response = client.post(RENDER, content=pages_of_text("a"), headers=AUTH, params=params)
    assert response.status_code == 400
    assert response.json() == {"error": "invalid request"}


@pytest.mark.parametrize(
    "params", [{"dpi": 100}, {"dpi": 300}, {"max_edge": 500}, {"max_edge": 2576}, {"page": 1}]
)
def test_the_limits_themselves_are_allowed(client: TestClient, params: dict[str, Any]) -> None:
    response = client.post(RENDER, content=one_page(""), headers=AUTH, params=params)
    assert response.status_code == 200


def test_an_empty_body_is_a_400(client: TestClient) -> None:
    response = client.post(RENDER, content=b"", headers=AUTH)
    assert response.status_code == 400
    assert response.json() == {"error": "the file is empty"}


def test_a_file_over_the_size_cap_is_refused_from_its_declared_length() -> None:
    small = TestClient(create_app(settings(max_request_bytes=100)))
    response = small.post(RENDER, content=b"%PDF-" + b"x" * 500, headers=AUTH)
    assert response.status_code == 413
    assert response.json() == {"error": "file too large", "max_bytes": 100}


# ---------- what the worker refuses ----------


def test_other_file_kinds_are_not_supported(client: TestClient) -> None:
    response = client.post(RENDER, content=b"<html>nope</html>", headers=AUTH)
    assert response.status_code == 415
    assert response.json() == {"error": "unsupported file type"}


def test_an_encrypted_pdf_is_a_422(client: TestClient) -> None:
    response = client.post(RENDER, content=make_encrypted_pdf(), headers=AUTH)
    assert response.status_code == 422
    assert response.json() == {"error": "the document is encrypted"}


def test_a_page_outside_the_document_says_how_many_pages_there_are(client: TestClient) -> None:
    pdf = pages_of_text("a", "b")
    response = client.post(RENDER, content=pdf, headers=AUTH, params={"page": 3})
    assert response.status_code == 422
    assert response.json() == {"error": "the page range is outside the document", "pages": 2}


def test_a_page_too_big_to_draw_is_a_422(client: TestClient) -> None:
    pdf = one_page("", media_box="[0 0 14401 100]")
    response = client.post(RENDER, content=pdf, headers=AUTH)
    assert response.status_code == 422
    assert response.json() == {"error": "the page is too large to render"}


def test_a_huge_picture_is_a_422(client: TestClient) -> None:
    response = client.post(RENDER, content=claims_to_be(30_000, 30_000), headers=AUTH)
    assert response.status_code == 422
    assert response.json() == {"error": "the image is too large"}


def test_a_damaged_document_is_a_422_that_reveals_nothing(client: TestClient) -> None:
    for body in (b"%PDF-1.4\nSECRET garbage", png_of(noise(40, 40))[:3000]):
        response = client.post(RENDER, content=body, headers=AUTH)
        assert response.status_code == 422
        assert response.json() == {"error": "the document could not be read"}
        assert "SECRET" not in response.text


def test_too_many_pages_is_a_422_that_names_the_cap() -> None:
    small = TestClient(create_app(settings(max_pages=2)))
    response = small.post(RENDER, content=pages_of_text("a", "b", "c"), headers=AUTH)
    assert response.status_code == 422
    assert response.json() == {"error": "too many pages", "max_pages": 2}


# ---------- how the app uses the renderer ----------


def canned() -> Any:
    def renderer(*args: Any, **kwargs: Any) -> EncodedImage:
        renderer.calls.append((args, kwargs))  # type: ignore[attr-defined]
        return EncodedImage(png(7, 5), "image/png", 7, 5)

    renderer.calls = []  # type: ignore[attr-defined]
    return renderer


def test_the_app_hands_the_renderer_what_was_asked_for_and_the_settings() -> None:
    renderer = canned()
    app = create_app(settings(max_pages=77, timeout_seconds=12.5), renderer=renderer)
    response = TestClient(app).post(
        RENDER,
        content=b"%PDF-1.4 bytes",
        headers=AUTH,
        params={"page": 3, "dpi": 200, "format": "jpeg", "max_edge": 1500},
    )
    assert response.status_code == 200
    ((args, kwargs),) = renderer.calls
    assert args == (b"%PDF-1.4 bytes", 3)
    assert kwargs == {
        "dpi": 200,
        "fmt": "jpeg",
        "max_edge": 1500,
        "max_pages": 77,
        "timeout_seconds": 12.5,
    }
    assert response.headers["x-image-width"] == "7"


def test_without_parameters_the_renderer_gets_the_defaults() -> None:
    renderer = canned()
    TestClient(create_app(settings(), renderer=renderer)).post(
        RENDER, content=b"%PDF-x", headers=AUTH
    )
    ((args, kwargs),) = renderer.calls
    assert args == (b"%PDF-x", 1)
    assert (kwargs["dpi"], kwargs["fmt"], kwargs["max_edge"]) == (150, "png", 2000)


@pytest.mark.parametrize(
    ("error", "status", "body"),
    [
        (NotAPdf(), 415, {"error": "unsupported file type"}),
        (Encrypted(), 422, {"error": "the document is encrypted"}),
        (Unreadable(), 422, {"error": "the document could not be read"}),
        (TooManyPages(9), 422, {"error": "too many pages", "max_pages": 9}),
        (TooSlow(), 504, {"error": "the document took too long to inspect"}),
        (PageTooBig(), 422, {"error": "the page is too large to render"}),
        (ImageTooLarge(), 422, {"error": "the image is too large"}),
        (
            PageOutOfRange(4),
            422,
            {"error": "the page range is outside the document", "pages": 4},
        ),
    ],
)
def test_whatever_the_renderer_refuses_with_is_translated(
    error: InspectError, status: int, body: dict[str, Any]
) -> None:
    def refuse(*args: Any, **kwargs: Any) -> EncodedImage:
        raise error

    response = TestClient(create_app(settings(max_pages=9), renderer=refuse)).post(
        RENDER, content=b"%PDF-x", headers=AUTH
    )
    assert (response.status_code, response.json()) == (status, body)


def test_logs_carry_sizes_and_outcomes_but_never_content_or_the_token(
    caplog: pytest.LogCaptureFixture, client: TestClient
) -> None:
    pdf = pages_of_text("SECRET invoice text")
    with caplog.at_level(logging.DEBUG, logger="clearcut_worker"):
        client.post(RENDER, content=pdf, headers=AUTH)
    assert "SECRET" not in caplog.text
    assert "tttttttt" not in caplog.text
    assert "/v1/render" in caplog.text
