# Written by Kone Tshivhinda & Claude
"""`POST /v1/text` over HTTP: who may ask, what it answers, and what it refuses."""

from __future__ import annotations

import logging
from typing import Any

import pytest
from fastapi.testclient import TestClient

from clearcut_worker.app import create_app
from clearcut_worker.inspect import (
    Encrypted,
    InspectError,
    NotAPdf,
    TooManyPages,
    TooSlow,
    Unreadable,
)
from clearcut_worker.pagetext import (
    FontFact,
    PageContent,
    PageOutOfRange,
    PageTooLarge,
    Word,
)

from .conftest import AUTH, make_encrypted_pdf, settings
from .pdfs import one_page, pages_of_text, text_at
from .pictures import jpeg, png

TEXT = "/v1/text"


@pytest.fixture
def client() -> TestClient:
    return TestClient(create_app(settings()))


def test_health_now_lists_text_among_the_things_the_worker_can_do(client: TestClient) -> None:
    assert client.get("/health").json()["capabilities"] == ["inspect", "text", "render"]


# ---------- authentication ----------


def test_anonymous_callers_get_the_same_401_before_anything_is_read(client: TestClient) -> None:
    response = client.post(TEXT, content=b"x" * 5000, params={"from": "banana"})
    assert response.status_code == 401
    assert response.json() == {"error": "authentication failed"}
    assert (
        client.post(TEXT, content=b"x", headers={"Authorization": "Bearer nope"}).status_code == 401
    )


# ---------- a successful read ----------


def test_a_pdf_is_read_page_by_page(client: TestClient) -> None:
    pdf = pages_of_text("Invoice INV-1", None, "Total 100.00")
    response = client.post(TEXT, content=pdf, headers=AUTH, params={"from": 1, "to": 3})
    assert response.status_code == 200
    pages = response.json()["pages"]
    assert [p["page"] for p in pages] == [1, 2, 3]
    assert pages[0]["source"] == "text_layer"
    assert "Invoice INV-1" in pages[0]["text"]
    assert pages[1]["source"] == "none"
    assert pages[1]["text"] == ""
    assert pages[0]["width_pt"] == 200.0
    assert pages[0]["fonts"] == [
        {"kind": "type1", "has_to_unicode": False, "encoding": "none", "embedded": False}
    ]
    assert "words" not in pages[0]


def test_words_come_back_only_when_asked_for(client: TestClient) -> None:
    pdf = one_page(text_at(20, 100, "Invoice INV-1"))
    plain = client.post(TEXT, content=pdf, headers=AUTH).json()["pages"][0]
    assert "words" not in plain
    asked = client.post(TEXT, content=pdf, headers=AUTH, params={"words": "true"}).json()["pages"][
        0
    ]
    assert len(asked["words"]) == 2
    assert set(asked["words"][0]) == {"start", "end", "x", "y", "w", "h"}


def test_leaving_out_the_page_range_reads_from_the_start_to_the_end(client: TestClient) -> None:
    pdf = pages_of_text("a", "b", "c")
    pages = client.post(TEXT, content=pdf, headers=AUTH).json()["pages"]
    assert [p["page"] for p in pages] == [1, 2, 3]
    from_two = client.post(TEXT, content=pdf, headers=AUTH, params={"from": 2}).json()["pages"]
    assert [p["page"] for p in from_two] == [2, 3]


# ---------- the request ----------


@pytest.mark.parametrize(
    "params",
    [
        {"from": 0},
        {"from": -1},
        {"from": "one"},
        {"from": 2, "to": 1},
        {"from": 1, "to": 51},
        {"from": 10, "to": 60},
        {"to": 0},
        {"to": "last"},
        {"words": "maybe"},
        {"from": 1.5},
    ],
)
def test_a_bad_page_range_or_flag_is_a_400_that_says_nothing_else(
    client: TestClient, params: dict[str, Any]
) -> None:
    response = client.post(TEXT, content=pages_of_text("a"), headers=AUTH, params=params)
    assert response.status_code == 400
    assert response.json() == {"error": "invalid request"}


def test_exactly_fifty_pages_in_one_call_is_allowed(client: TestClient) -> None:
    pdf = pages_of_text(*["x"] * 50)
    response = client.post(TEXT, content=pdf, headers=AUTH, params={"from": 1, "to": 50})
    assert response.status_code == 200
    assert len(response.json()["pages"]) == 50


def test_an_empty_body_is_a_400(client: TestClient) -> None:
    response = client.post(TEXT, content=b"", headers=AUTH)
    assert response.status_code == 400
    assert response.json() == {"error": "the file is empty"}


def test_a_file_over_the_size_cap_is_refused_from_its_declared_length() -> None:
    small = TestClient(create_app(settings(max_request_bytes=100)))
    response = small.post(TEXT, content=b"%PDF-" + b"x" * 500, headers=AUTH)
    assert response.status_code == 413
    assert response.json() == {"error": "file too large", "max_bytes": 100}


# ---------- what the worker refuses ----------


def test_other_file_kinds_are_not_supported(client: TestClient) -> None:
    response = client.post(TEXT, content=b"<html>nope</html>", headers=AUTH)
    assert response.status_code == 415
    assert response.json() == {"error": "unsupported file type"}


def test_an_encrypted_pdf_is_a_422(client: TestClient) -> None:
    response = client.post(TEXT, content=make_encrypted_pdf(), headers=AUTH)
    assert response.status_code == 422
    assert response.json() == {"error": "the document is encrypted"}


def test_a_page_range_outside_the_document_is_a_422_that_says_how_many_pages_there_are(
    client: TestClient,
) -> None:
    response = client.post(
        TEXT, content=pages_of_text("a", "b"), headers=AUTH, params={"from": 2, "to": 5}
    )
    assert response.status_code == 422
    assert response.json() == {"error": "the page range is outside the document", "pages": 2}


def test_a_damaged_pdf_is_a_422_that_reveals_nothing_about_the_parser(client: TestClient) -> None:
    response = client.post(TEXT, content=b"%PDF-1.4\nSECRET garbage", headers=AUTH)
    assert response.status_code == 422
    assert response.json() == {"error": "the document could not be read"}
    assert "SECRET" not in response.text


def test_too_many_pages_is_a_422_that_names_the_cap() -> None:
    small = TestClient(create_app(settings(max_pages=2)))
    response = small.post(TEXT, content=pages_of_text("a", "b", "c"), headers=AUTH)
    assert response.status_code == 422
    assert response.json() == {"error": "too many pages", "max_pages": 2}


def test_no_error_ever_echoes_the_request_content(client: TestClient) -> None:
    for body in [b"SECRET", b"%PDF-SECRET", b"%PDF-1.4\nSECRET" * 10]:
        response = client.post(TEXT, content=body, headers=AUTH)
        assert "SECRET" not in response.text


# ---------- how the app uses the reader ----------


def canned(*pages: PageContent) -> Any:
    def reader(*args: Any, **kwargs: Any) -> list[PageContent]:
        reader.calls.append((args, kwargs))  # type: ignore[attr-defined]
        return list(pages)

    reader.calls = []  # type: ignore[attr-defined]
    return reader


def one(number: int = 1) -> PageContent:
    return PageContent(
        page=number,
        source="text_layer",
        text="Total 100",
        width_pt=595.0,
        height_pt=842.0,
        fonts=(FontFact("truetype", True, "identity", True),),
        words=(Word(0, 5, 10, 20, 30, 40),),
    )


def test_the_app_hands_the_reader_the_bytes_the_range_the_flag_the_cap_and_the_timeout() -> None:
    reader = canned(one(2), one(3))
    app = create_app(settings(max_pages=77, timeout_seconds=12.5), text_reader=reader)
    response = TestClient(app).post(
        TEXT, content=b"%PDF-1.4 bytes", headers=AUTH, params={"from": 2, "to": 3, "words": "true"}
    )
    assert response.status_code == 200
    ((args, kwargs),) = reader.calls
    assert args == (b"%PDF-1.4 bytes", 2, 3)
    assert kwargs == {"words": True, "max_pages": 77, "timeout_seconds": 12.5}
    page = response.json()["pages"][0]
    assert page["fonts"] == [
        {"kind": "truetype", "has_to_unicode": True, "encoding": "identity", "embedded": True}
    ]
    assert page["words"] == [{"start": 0, "end": 5, "x": 10, "y": 20, "w": 30, "h": 40}]


def test_an_open_ended_range_reaches_the_reader_as_none() -> None:
    reader = canned(one())
    TestClient(create_app(settings(), text_reader=reader)).post(
        TEXT, content=b"%PDF-x", headers=AUTH
    )
    ((args, _),) = reader.calls
    assert args == (b"%PDF-x", 1, None)


@pytest.mark.parametrize(
    ("error", "status", "body"),
    [
        (NotAPdf(), 415, {"error": "unsupported file type"}),
        (Encrypted(), 422, {"error": "the document is encrypted"}),
        (Unreadable(), 422, {"error": "the document could not be read"}),
        (TooManyPages(9), 422, {"error": "too many pages", "max_pages": 9}),
        (TooSlow(), 504, {"error": "the document took too long to inspect"}),
        (PageTooLarge(), 422, {"error": "a page holds more text than the worker will report"}),
        (
            PageOutOfRange(4),
            422,
            {"error": "the page range is outside the document", "pages": 4},
        ),
    ],
)
def test_whatever_the_reader_refuses_with_is_translated(
    error: InspectError, status: int, body: dict[str, Any]
) -> None:
    def refuse(*args: Any, **kwargs: Any) -> list[PageContent]:
        raise error

    response = TestClient(create_app(settings(max_pages=9), text_reader=refuse)).post(
        TEXT, content=b"%PDF-x", headers=AUTH
    )
    assert (response.status_code, response.json()) == (status, body)


def test_logs_carry_sizes_and_outcomes_but_never_text_or_the_token(
    caplog: pytest.LogCaptureFixture, client: TestClient
) -> None:
    pdf = pages_of_text("SECRET invoice text")
    with caplog.at_level(logging.DEBUG, logger="clearcut_worker"):
        client.post(TEXT, content=pdf, headers=AUTH, params={"words": "true"})
    assert "SECRET" not in caplog.text
    assert "tttttttt" not in caplog.text
    assert "/v1/text" in caplog.text


# ---------- a picture as the document ----------


def test_a_picture_is_one_page_with_no_text_measured_in_pixels(client: TestClient) -> None:
    response = client.post(TEXT, content=png(300, 200), headers=AUTH, params={"words": "true"})
    assert response.status_code == 200
    [page] = response.json()["pages"]
    assert page["page"] == 1
    assert page["source"] == "none"
    assert page["text"] == ""
    assert (page["width_pt"], page["height_pt"]) == (300.0, 200.0)
    assert page["fonts"] == []
    assert page["words"] == []


def test_a_photo_is_measured_upright(client: TestClient) -> None:
    [page] = client.post(TEXT, content=jpeg(200, 100, orientation=6), headers=AUTH).json()["pages"]
    assert (page["width_pt"], page["height_pt"]) == (100.0, 200.0)


def test_asking_a_picture_for_a_second_page_says_it_has_one(client: TestClient) -> None:
    response = client.post(TEXT, content=png(), headers=AUTH, params={"from": 2})
    assert response.status_code == 422
    assert response.json() == {"error": "the page range is outside the document", "pages": 1}
