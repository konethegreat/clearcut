# Written by Kone Tshivhinda & Claude
"""`POST /v1/ocr` over HTTP: who may ask, what it answers, and what it refuses."""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Any

import pytest
from fastapi.testclient import TestClient

from clearcut_worker.app import create_app
from clearcut_worker.images import ImageTooLarge
from clearcut_worker.inspect import (
    Encrypted,
    InspectError,
    NotAPdf,
    TooManyPages,
    TooSlow,
    Unreadable,
)
from clearcut_worker.ocr import OcrEngine
from clearcut_worker.pagetext import PageContent, PageOutOfRange, PageTooLarge, Word
from clearcut_worker.render import PageTooBig

from .conftest import AUTH, make_encrypted_pdf, settings
from .ocrs import fake_engine, row, tsv
from .pdfs import one_page, pages_of_text, text_at

OCR = "/v1/ocr"
ENGINE = OcrEngine(("tesseract",), None)


def a_page(**overrides: Any) -> PageContent:
    base: dict[str, Any] = {
        "page": 1,
        "source": "ocr",
        "text": "Total 100",
        "width_pt": 595.0,
        "height_pt": 842.0,
        "fonts": (),
        "words": (Word(0, 5, 10, 20, 100, 30, 900), Word(6, 9, 120, 20, 100, 30, 850)),
        "agreement": None,
    }
    base.update(overrides)
    return PageContent(**base)


def canned(page: PageContent | None = None) -> Any:
    def reader(*args: Any, **kwargs: Any) -> PageContent:
        reader.calls.append((args, kwargs))  # type: ignore[attr-defined]
        return page or a_page()

    reader.calls = []  # type: ignore[attr-defined]
    return reader


@pytest.fixture
def client() -> TestClient:
    return TestClient(create_app(settings(), ocr_engine=ENGINE, ocr_reader=canned()))


# ---------- whether OCR is on ----------


def test_health_lists_ocr_only_when_there_is_an_engine() -> None:
    with_engine = TestClient(create_app(settings(), ocr_engine=ENGINE))
    assert with_engine.get("/health").json()["capabilities"] == ["inspect", "text", "render", "ocr"]
    without = TestClient(create_app(settings()))
    assert without.get("/health").json()["capabilities"] == ["inspect", "text", "render"]


def test_without_an_engine_ocr_is_a_503_that_says_so_after_the_key_is_checked() -> None:
    off = TestClient(create_app(settings()))
    response = off.post(OCR, content=pages_of_text("a"), headers=AUTH)
    assert response.status_code == 503
    assert response.json() == {"error": "OCR is not available"}
    assert off.post(OCR, content=b"x").status_code == 401


# ---------- authentication ----------


def test_anonymous_callers_get_the_same_401_before_anything_is_read(client: TestClient) -> None:
    response = client.post(OCR, content=b"x" * 5000, params={"page": "banana"})
    assert response.status_code == 401
    assert response.json() == {"error": "authentication failed"}
    assert (
        client.post(OCR, content=b"x", headers={"Authorization": "Bearer nope"}).status_code == 401
    )


# ---------- a successful reading ----------


def test_a_page_comes_back_in_the_shape_of_the_text_answer_with_confidence(
    client: TestClient,
) -> None:
    response = client.post(OCR, content=pages_of_text("a"), headers=AUTH)
    assert response.status_code == 200
    assert response.json() == {
        "pages": [
            {
                "page": 1,
                "source": "ocr",
                "text": "Total 100",
                "width_pt": 595.0,
                "height_pt": 842.0,
                "fonts": [],
                "words": [
                    {"start": 0, "end": 5, "x": 10, "y": 20, "w": 100, "h": 30, "confidence": 900},
                    {"start": 6, "end": 9, "x": 120, "y": 20, "w": 100, "h": 30, "confidence": 850},
                ],
            }
        ]
    }


def test_the_agreement_appears_only_when_it_was_asked_for_and_worked_out() -> None:
    reader = canned(a_page(agreement=730))
    app = TestClient(create_app(settings(), ocr_engine=ENGINE, ocr_reader=reader))
    asked = app.post(OCR, content=b"%PDF-x", headers=AUTH, params={"agreement": "true"})
    assert asked.json()["pages"][0]["agreement"] == 730
    plain = TestClient(create_app(settings(), ocr_engine=ENGINE, ocr_reader=canned(a_page())))
    assert "agreement" not in plain.post(OCR, content=b"%PDF-x", headers=AUTH).json()["pages"][0]


def test_an_agreement_that_could_not_be_worked_out_is_left_out_not_zero() -> None:
    app = TestClient(create_app(settings(), ocr_engine=ENGINE, ocr_reader=canned(a_page())))
    page = app.post(OCR, content=b"%PDF-x", headers=AUTH, params={"agreement": "true"}).json()
    assert "agreement" not in page["pages"][0]


def test_a_zero_agreement_is_reported_as_zero() -> None:
    reader = canned(a_page(agreement=0))
    app = TestClient(create_app(settings(), ocr_engine=ENGINE, ocr_reader=reader))
    page = app.post(OCR, content=b"%PDF-x", headers=AUTH, params={"agreement": "true"}).json()
    assert page["pages"][0]["agreement"] == 0


# ---------- the request ----------


@pytest.mark.parametrize(
    "params",
    [
        {"page": 0},
        {"page": -1},
        {"page": "one"},
        {"page": 1.5},
        {"agreement": "maybe"},
        {"agreement": ""},
    ],
)
def test_a_bad_parameter_is_a_400_that_says_nothing_else(
    client: TestClient, params: dict[str, Any]
) -> None:
    response = client.post(OCR, content=pages_of_text("a"), headers=AUTH, params=params)
    assert response.status_code == 400
    assert response.json() == {"error": "invalid request"}


def test_an_empty_body_is_a_400(client: TestClient) -> None:
    response = client.post(OCR, content=b"", headers=AUTH)
    assert response.status_code == 400
    assert response.json() == {"error": "the file is empty"}


def test_a_file_over_the_size_cap_is_refused_from_its_declared_length() -> None:
    small = TestClient(
        create_app(settings(max_request_bytes=100), ocr_engine=ENGINE, ocr_reader=canned())
    )
    response = small.post(OCR, content=b"%PDF-" + b"x" * 500, headers=AUTH)
    assert response.status_code == 413
    assert response.json() == {"error": "file too large", "max_bytes": 100}


# ---------- how the app uses the reader ----------


def test_the_app_hands_the_reader_the_bytes_the_page_the_engine_and_the_settings() -> None:
    reader = canned()
    engine = OcrEngine(("/opt/t",), "/opt/data")
    app = create_app(
        settings(max_pages=77, timeout_seconds=12.5), ocr_engine=engine, ocr_reader=reader
    )
    TestClient(app).post(
        OCR, content=b"%PDF-1.4 bytes", headers=AUTH, params={"page": 3, "agreement": "true"}
    )
    ((args, kwargs),) = reader.calls
    assert args == (b"%PDF-1.4 bytes", 3)
    assert kwargs == {
        "engine": engine,
        "agreement": True,
        "max_pages": 77,
        "timeout_seconds": 12.5,
    }


def test_without_parameters_it_reads_page_one_and_works_out_no_agreement() -> None:
    reader = canned()
    TestClient(create_app(settings(), ocr_engine=ENGINE, ocr_reader=reader)).post(
        OCR, content=b"%PDF-x", headers=AUTH
    )
    ((args, kwargs),) = reader.calls
    assert args == (b"%PDF-x", 1)
    assert kwargs["agreement"] is False


@pytest.mark.parametrize(
    ("error", "status", "body"),
    [
        (NotAPdf(), 415, {"error": "unsupported file type"}),
        (Encrypted(), 422, {"error": "the document is encrypted"}),
        (Unreadable(), 422, {"error": "the document could not be read"}),
        (TooManyPages(9), 422, {"error": "too many pages", "max_pages": 9}),
        (TooSlow(), 504, {"error": "the document took too long to inspect"}),
        (PageTooLarge(), 422, {"error": "a page holds more text than the worker will report"}),
        (PageTooBig(), 422, {"error": "the page is too large to render"}),
        (ImageTooLarge(), 422, {"error": "the image is too large"}),
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
    def refuse(*args: Any, **kwargs: Any) -> PageContent:
        raise error

    app = create_app(settings(max_pages=9), ocr_engine=ENGINE, ocr_reader=refuse)
    response = TestClient(app).post(OCR, content=b"%PDF-x", headers=AUTH)
    assert (response.status_code, response.json()) == (status, body)


def test_logs_carry_sizes_and_outcomes_but_never_text_or_the_token(
    caplog: pytest.LogCaptureFixture, client: TestClient
) -> None:
    with caplog.at_level(logging.DEBUG, logger="clearcut_worker"):
        client.post(OCR, content=pages_of_text("SECRET invoice text"), headers=AUTH)
    assert "SECRET" not in caplog.text
    assert "Total 100" not in caplog.text
    assert "tttttttt" not in caplog.text
    assert "/v1/ocr" in caplog.text


# ---------- all the way through, with the stand-in program ----------


def test_a_real_request_runs_the_whole_chain_with_the_stand_in_program(tmp_path: Path) -> None:
    engine = fake_engine(
        tmp_path / "engine",
        tsv=tsv(
            row("Invoice", 83, 380, 150, 40, 96.5), row("INV-1", 250, 380, 100, 40, 88.0, word=2)
        ),
    )
    app = TestClient(create_app(settings(), ocr_engine=engine))
    response = app.post(
        OCR,
        content=one_page(text_at(20, 100, "Invoice INV-1")),
        headers=AUTH,
        params={"agreement": "true"},
    )
    assert response.status_code == 200
    [page] = response.json()["pages"]
    assert page["source"] == "ocr"
    assert page["text"] == "Invoice INV-1"
    assert page["agreement"] == 1000
    assert [w["confidence"] for w in page["words"]] == [965, 880]


def test_a_stand_in_program_that_fails_is_a_422_that_reveals_nothing(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine", exit=1, stderr="SECRET from the program")
    app = TestClient(create_app(settings(), ocr_engine=engine))
    response = app.post(OCR, content=pages_of_text("a"), headers=AUTH)
    assert response.status_code == 422
    assert response.json() == {"error": "the document could not be read"}
    assert "SECRET" not in response.text
    assert make_encrypted_pdf()  # keeps the helper import honest
