# Written by Kone Tshivhinda & Claude
"""Looking inside a PDF, without trusting it."""

from __future__ import annotations

import time

import pytest

from clearcut_worker.images import ImageTooLarge
from clearcut_worker.inspect import (
    Encrypted,
    Inspection,
    NotAPdf,
    TooManyPages,
    TooSlow,
    Unreadable,
    inspect_pdf,
)

from .conftest import make_encrypted_pdf, make_pdf
from .pictures import claims_to_be, jpeg, noise, png, png_of


def far_deadline() -> float:
    return time.monotonic() + 60


def test_a_document_with_text_on_every_page() -> None:
    result = inspect_pdf(make_pdf(["Invoice 1", "Invoice 2", "Invoice 3"]), 50, far_deadline())
    assert result == Inspection(pages=3, pages_with_text=3)
    assert result.has_text_layer is True


def test_blank_pages_have_no_text_layer() -> None:
    result = inspect_pdf(make_pdf([None, None]), 50, far_deadline())
    assert result == Inspection(pages=2, pages_with_text=0)
    assert result.has_text_layer is False


def test_a_mixed_document_counts_only_the_pages_with_text() -> None:
    result = inspect_pdf(make_pdf(["Total R100", None, "Page three", None]), 50, far_deadline())
    assert (result.pages, result.pages_with_text) == (4, 2)
    assert result.has_text_layer is True


def test_a_page_of_only_whitespace_does_not_count_as_text() -> None:
    result = inspect_pdf(make_pdf(["   "]), 50, far_deadline())
    assert result.pages_with_text == 0


def test_exactly_the_page_cap_is_allowed_and_one_more_is_not() -> None:
    assert inspect_pdf(make_pdf(["a"] * 5), 5, far_deadline()).pages == 5
    with pytest.raises(TooManyPages) as caught:
        inspect_pdf(make_pdf(["a"] * 6), 5, far_deadline())
    assert caught.value.max_pages == 5


def test_something_that_is_not_a_pdf_is_refused_by_what_the_bytes_say() -> None:
    for blob in [b"", b"hello", b"GIF89a....", b"PK\x03\x04zip", b"<xml/>", b"%PD"]:
        with pytest.raises(NotAPdf):
            inspect_pdf(blob, 50, far_deadline())
    # A PNG signature with nothing real behind it is a PNG, and an unreadable one.
    with pytest.raises(Unreadable):
        inspect_pdf(b"\x89PNG\r\n\x1a\n....", 50, far_deadline())


def test_a_pdf_header_is_found_at_the_start_only() -> None:
    with pytest.raises(NotAPdf):
        inspect_pdf(b" " + make_pdf(["x"]), 50, far_deadline())


def test_a_damaged_pdf_is_unreadable() -> None:
    good = make_pdf(["Invoice"])
    for blob in [
        b"%PDF-1.4\nthis is not a pdf at all",
        good[: len(good) // 2],
        b"%PDF-1.7\n" + b"\x00" * 200,
    ]:
        with pytest.raises(Unreadable):
            inspect_pdf(blob, 50, far_deadline())


def test_an_encrypted_pdf_is_refused_not_opened() -> None:
    with pytest.raises(Encrypted):
        inspect_pdf(make_encrypted_pdf(), 50, far_deadline())


def test_it_gives_up_when_the_deadline_has_passed() -> None:
    with pytest.raises(TooSlow):
        inspect_pdf(make_pdf(["a", "b"]), 50, time.monotonic() - 1)


def test_the_error_messages_never_contain_document_content() -> None:
    secret = b"%PDF-1.4\nSECRET-PAYLOAD 1234567890 not really a pdf"
    with pytest.raises(Unreadable) as caught:
        inspect_pdf(secret, 50, far_deadline())
    assert "SECRET" not in str(caught.value)
    assert "1234567890" not in str(caught.value)
    assert caught.value.__cause__ is None or "SECRET" not in str(caught.value.__cause__)


def test_the_deadline_is_inclusive(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("clearcut_worker.inspect.time.monotonic", lambda: 100.0)
    pdf = make_pdf(["a"])
    with pytest.raises(TooSlow):
        inspect_pdf(pdf, 50, 100.0)
    assert inspect_pdf(pdf, 50, 100.5).pages == 1


def test_a_page_whose_text_cannot_be_read_counts_as_having_none(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from pypdf import PageObject

    original = PageObject.extract_text
    calls = {"n": 0}

    def flaky(self: PageObject, *args: object, **kwargs: object) -> str:
        calls["n"] += 1
        if calls["n"] == 2:
            raise ValueError("SECRET broken content stream")
        return original(self, *args, **kwargs)  # type: ignore[arg-type]

    monkeypatch.setattr(PageObject, "extract_text", flaky)
    result = inspect_pdf(make_pdf(["one", "two", "three"]), 50, far_deadline())
    assert (result.pages, result.pages_with_text) == (3, 2)


def test_a_single_page_with_text_is_a_text_layer() -> None:
    assert Inspection(pages=1, pages_with_text=1).has_text_layer is True
    assert Inspection(pages=1, pages_with_text=0).has_text_layer is False
    assert Inspection(pages=9, pages_with_text=1).has_text_layer is True


# ---------- a picture is a one-page document ----------


def test_a_pdf_says_it_is_one() -> None:
    assert inspect_pdf(make_pdf(["a"]), 50, far_deadline()).media_type == "application/pdf"


def test_a_png_or_a_jpeg_is_a_one_page_document_with_no_text_layer() -> None:
    assert inspect_pdf(png(), 50, far_deadline()) == Inspection(
        pages=1, pages_with_text=0, media_type="image/png"
    )
    result = inspect_pdf(jpeg(), 50, far_deadline())
    assert result == Inspection(pages=1, pages_with_text=0, media_type="image/jpeg")
    assert result.has_text_layer is False


def test_a_picture_is_really_decoded_so_a_damaged_one_is_unreadable() -> None:
    with pytest.raises(Unreadable):
        inspect_pdf(png_of(noise(40, 40))[:3000], 50, far_deadline())


def test_a_picture_that_claims_to_be_huge_is_refused() -> None:
    with pytest.raises(ImageTooLarge):
        inspect_pdf(claims_to_be(30_000, 30_000), 50, far_deadline())


def test_the_page_cap_does_not_stop_a_picture() -> None:
    assert inspect_pdf(png(), 1, far_deadline()).pages == 1


def test_a_picture_gets_the_deadline_check_too() -> None:
    with pytest.raises(TooSlow):
        inspect_pdf(png(), 50, time.monotonic() - 1)
