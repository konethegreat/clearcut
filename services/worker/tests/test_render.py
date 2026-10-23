# Written by Kone Tshivhinda & Claude
"""Rendering a page, or a picture, into the image the model will be shown (ADR-0016)."""

from __future__ import annotations

import io
import time
from typing import Any

import pytest
from PIL import Image

from clearcut_worker.images import EncodedImage, ImageTooLarge
from clearcut_worker.inspect import Encrypted, NotAPdf, TooManyPages, TooSlow, Unreadable
from clearcut_worker.pagetext import PageOutOfRange
from clearcut_worker.render import (
    DEFAULT_DPI,
    MAX_DPI,
    MAX_PAGE_POINTS,
    MIN_DPI,
    PageTooBig,
    render_page,
)

from .conftest import make_encrypted_pdf
from .pdfs import PageSpec, PdfBuilder, one_page, pages_of_text, text_at
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
from .test_pagetext import FakePdfiumDocument

FAR = time.monotonic() + 3600
BLACK = (0, 0, 0)
WHITE = (255, 255, 255)
# A filled black square, 100 points on a side, from (50, 50) to (150, 150) of the page.
SQUARE = "0 0 0 rg 50 50 100 100 re f\n"


def render(data: bytes, page: int = 1, **options: Any) -> EncodedImage:
    settings: dict[str, Any] = {"max_pages": 50, "deadline": FAR}
    settings.update(options)
    return render_page(data, page, **settings)


def test_the_limits_are_the_ones_the_design_names() -> None:
    assert (MIN_DPI, DEFAULT_DPI, MAX_DPI) == (100, 150, 300)
    assert MAX_PAGE_POINTS == 14_400


# ---------- a page of a PDF ----------


def test_a_page_is_drawn_where_it_says_at_the_dpi_asked_for() -> None:
    result = render(one_page(SQUARE), dpi=144)  # two pixels to the point
    assert result.media_type == "image/png"
    assert (result.width, result.height) == (400, 400)
    assert size_of(result.data) == (400, 400)
    assert pixel(result.data, 200, 200) == BLACK  # the middle of the square
    assert pixel(result.data, 20, 20) == WHITE  # outside it
    assert pixel(result.data, 380, 380) == WHITE
    assert pixel(result.data, 110, 290) == BLACK  # inside, near the bottom left of the square
    assert pixel(result.data, 90, 290) == WHITE  # just outside it


def test_an_a4_page_at_the_default_dpi_is_about_1240_by_1754() -> None:
    result = render(one_page("", media_box="[0 0 595 842]"))
    assert abs(result.width - 1240) <= 1
    assert abs(result.height - 1754) <= 1


def test_a_long_edge_over_the_cap_is_scaled_down_whatever_the_dpi() -> None:
    result = render(one_page("", media_box="[0 0 595 842]"), dpi=300)
    assert result.height == 2000
    assert abs(result.width - 595 * 2000 / 842) <= 1
    wide = render(one_page("", media_box="[0 0 595 842]"), dpi=300, max_edge=2576)
    assert wide.height == 2576


def test_the_page_comes_out_white_where_nothing_is_drawn() -> None:
    result = render(one_page(""), dpi=100)
    assert pixel(result.data, 5, 5) == WHITE
    assert pixel(result.data, result.width - 5, result.height - 5) == WHITE


def test_text_that_uses_a_font_the_pdf_does_not_carry_is_still_drawn() -> None:
    result = render(one_page(text_at(20, 100, "Invoice INV-1", size=24)), dpi=150)
    with Image.open(io.BytesIO(result.data)) as image:
        dark = sum(image.convert("L").histogram()[:128])
    assert dark > 100


def test_a_turned_page_is_drawn_the_way_a_reader_sees_it() -> None:
    # 200 by 100 points, turned a quarter: 100 wide and 200 tall. A square in its bottom
    # left corner (before turning) ends up in the top left.
    pdf = one_page("0 0 0 rg 0 0 20 20 re f\n", media_box="[0 0 200 100]", rotate=90)
    result = render(pdf, dpi=144)
    assert (result.width, result.height) == (200, 400)
    assert pixel(result.data, 10, 10) == BLACK
    assert pixel(result.data, 190, 390) == WHITE


def test_only_the_crop_box_is_drawn() -> None:
    outside = "0 0 0 rg 0 0 40 40 re f\n"
    inside = "0 0 0 rg 60 60 40 40 re f\n"
    pdf = one_page(outside + inside, media_box="[0 0 200 200]", crop_box="[50 50 150 150]")
    result = render(pdf, dpi=144)
    assert (result.width, result.height) == (200, 200)
    assert pixel(result.data, 10, 190) == WHITE  # where the first square would have been
    assert pixel(result.data, 60, 140) == BLACK  # the second, 10 points in from the corner


def test_a_later_page_of_a_document_can_be_asked_for() -> None:
    pdf = PdfBuilder(
        pages=[PageSpec(content=""), PageSpec(content=SQUARE), PageSpec(content="")]
    ).build()
    assert pixel(render(pdf, 2, dpi=144).data, 200, 200) == BLACK
    assert pixel(render(pdf, 1, dpi=144).data, 200, 200) == WHITE


def test_asking_for_a_jpeg_gives_a_jpeg() -> None:
    result = render(one_page(SQUARE), fmt="jpeg")
    assert result.media_type == "image/jpeg"
    assert result.data.startswith(b"\xff\xd8\xff")


def test_the_picture_carries_no_metadata_at_all() -> None:
    result = render(one_page(SQUARE))
    assert b"tEXt" not in result.data
    assert b"iCCP" not in result.data


# ---------- what is refused ----------


def test_a_page_outside_the_document_says_how_many_pages_there_are() -> None:
    with pytest.raises(PageOutOfRange) as caught:
        render(pages_of_text("a", "b"), 3)
    assert caught.value.pages == 2


def test_page_zero_is_a_programming_error() -> None:
    for page in (0, -1):
        with pytest.raises(ValueError, match="page"):
            render(pages_of_text("a"), page)


def test_what_cannot_be_read_is_refused_in_a_named_way() -> None:
    with pytest.raises(NotAPdf):
        render(b"<html>nope</html>")
    with pytest.raises(Encrypted):
        render(make_encrypted_pdf())
    with pytest.raises(Unreadable):
        render(b"%PDF-1.4\nSECRET garbage")
    with pytest.raises(TooManyPages):
        render(pages_of_text("a", "b", "c"), max_pages=2)


def test_a_page_wider_than_fourteen_thousand_four_hundred_points_is_refused() -> None:
    with pytest.raises(PageTooBig):
        render(one_page("", media_box="[0 0 14401 100]"), dpi=100)
    with pytest.raises(PageTooBig):
        render(one_page("", media_box="[0 0 100 14401]"), dpi=100)
    # Exactly the limit is drawn (and then brought down to the long-edge cap).
    assert render(one_page("", media_box="[0 0 14400 100]"), dpi=100).width == 2000


def test_a_page_that_would_need_too_many_pixels_is_refused_before_it_is_drawn() -> None:
    pdf = one_page("", media_box="[0 0 200 200]")
    with pytest.raises(PageTooBig):
        render(pdf, dpi=72, max_pixels=39_999)
    assert render(pdf, dpi=72, max_pixels=40_000).width == 200


def test_a_deadline_that_has_passed_stops_it_before_the_engine_is_opened() -> None:
    opened: list[bytes] = []

    def opener(data: bytes) -> FakePdfiumDocument:
        opened.append(data)
        return FakePdfiumDocument(1)

    with pytest.raises(TooSlow):
        render(pages_of_text("a"), deadline=time.monotonic() - 1, pdfium_open=opener)
    assert opened == []


def test_two_parsers_that_disagree_about_the_page_count_mean_an_unreadable_document() -> None:
    fake = FakePdfiumDocument(5)
    with pytest.raises(Unreadable):
        render(pages_of_text("a", "b"), 1, pdfium_open=lambda data: fake)
    assert fake.closed


class ExplodingPage:
    def get_size(self) -> tuple[float, float]:
        return (200.0, 200.0)

    def render(self, **options: object) -> object:
        raise RuntimeError("SECRET internals")

    def close(self) -> None:
        pass


class ExplodingDocument(FakePdfiumDocument):
    def __getitem__(self, index: int) -> Any:
        return ExplodingPage()


def test_an_engine_that_fails_while_drawing_is_an_unreadable_document_without_a_message() -> None:
    document = ExplodingDocument(1)
    with pytest.raises(Unreadable) as caught:
        render(pages_of_text("a"), pdfium_open=lambda data: document)
    assert "SECRET" not in str(caught.value)
    assert document.closed


# ---------- a picture as a document ----------


def test_a_png_document_is_its_own_one_page() -> None:
    source = png_of(noise(40, 30))
    result = render(source)
    assert result.media_type == "image/png"
    assert (result.width, result.height) == (40, 30)
    with Image.open(io.BytesIO(result.data)) as again, Image.open(io.BytesIO(source)) as first:
        assert again.tobytes() == first.convert("RGB").tobytes()


def test_the_dpi_means_nothing_to_a_picture() -> None:
    assert render(png(40, 30), dpi=100).data == render(png(40, 30), dpi=300).data


def test_a_big_picture_is_brought_down_to_the_long_edge_cap() -> None:
    result = render(png(3000, 1000))
    assert (result.width, result.height) == (2000, 667)


def test_a_photo_comes_back_upright_and_without_where_it_was_taken() -> None:
    photo = jpeg(200, 100, orientation=6, with_metadata=True)
    result = render(photo)
    assert (result.width, result.height) == (100, 200)
    assert EXIF_MARKER.encode() not in result.data
    assert COMMENT_MARKER not in result.data
    assert b"Exif" not in result.data


def test_only_page_one_of_a_picture_exists() -> None:
    with pytest.raises(PageOutOfRange) as caught:
        render(png(), 2)
    assert caught.value.pages == 1


def test_a_picture_that_claims_to_be_huge_is_refused() -> None:
    with pytest.raises(ImageTooLarge):
        render(claims_to_be(30_000, 30_000))


def test_a_damaged_picture_is_unreadable() -> None:
    with pytest.raises(Unreadable):
        render(png_of(noise(40, 40))[:3000])


def test_a_picture_gets_the_same_deadline_check() -> None:
    with pytest.raises(TooSlow):
        render(png(), deadline=time.monotonic() - 1)


class RecordingPage:
    def __init__(self) -> None:
        self.render_options: dict[str, object] = {}
        self.closes = 0

    def get_size(self) -> tuple[float, float]:
        return (200.0, 100.0)

    def render(self, **options: object) -> Any:
        self.render_options = options
        return FakeBitmap(Image.new("RGB", (10, 5), (255, 255, 255)))

    def close(self) -> None:
        self.closes += 1


class OnePageDocument(FakePdfiumDocument):
    def __init__(self, page: Any) -> None:
        super().__init__(1)
        self.page = page

    def __getitem__(self, index: int) -> Any:
        return self.page


class FakeBitmap:
    def __init__(self, picture: Image.Image) -> None:
        self.picture = picture

    def to_pil(self) -> Image.Image:
        return self.picture


def test_forms_are_not_drawn_and_the_page_and_document_are_closed() -> None:
    page = RecordingPage()
    document = OnePageDocument(page)
    render(pages_of_text("a"), dpi=144, pdfium_open=lambda data: document)
    assert page.render_options == {"scale": 2.0, "may_draw_forms": False}
    assert page.closes == 1
    assert document.closed


class SizedPage(RecordingPage):
    def __init__(self, size: tuple[float, float]) -> None:
        super().__init__()
        self.size = size

    def get_size(self) -> tuple[float, float]:
        return self.size


@pytest.mark.parametrize(
    "size",
    [
        (0.0, 100.0),
        (100.0, -1.0),
        (float("nan"), 100.0),
        (100.0, float("nan")),
        (float("inf"), 5.0),
    ],
)
def test_a_page_with_a_nonsense_size_is_unreadable(size: tuple[float, float]) -> None:
    page = SizedPage(size)
    document = OnePageDocument(page)
    with pytest.raises(Unreadable):
        render(pages_of_text("a"), pdfium_open=lambda data: document)
    assert page.closes == 1
    assert document.closed


def test_a_picture_is_refused_when_it_is_over_the_pixel_limit_given() -> None:
    assert render(png(10, 10), max_pixels=100).width == 10
    with pytest.raises(ImageTooLarge):
        render(png(11, 10), max_pixels=100)


def test_a_picture_is_not_even_opened_when_the_time_is_already_up(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def must_not_open(*args: object, **kwargs: object) -> None:
        raise AssertionError("opened a picture after the deadline")

    monkeypatch.setattr("clearcut_worker.render.open_image", must_not_open)
    with pytest.raises(TooSlow):
        render(png(), deadline=time.monotonic() - 1)


def test_a_picture_that_takes_too_long_to_open_is_too_slow(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    real = __import__("clearcut_worker.render", fromlist=["open_image"]).open_image

    def slow_open(*args: Any, **kwargs: Any) -> Any:
        time.sleep(0.3)
        return real(*args, **kwargs)

    monkeypatch.setattr("clearcut_worker.render.open_image", slow_open)
    with pytest.raises(TooSlow):
        render(png(), deadline=time.monotonic() + 0.15)


def test_a_page_is_not_drawn_when_the_time_ran_out_while_the_engine_was_opening() -> None:
    page = RecordingPage()
    document = OnePageDocument(page)

    def slow_open(data: bytes) -> OnePageDocument:
        time.sleep(0.3)
        return document

    with pytest.raises(TooSlow):
        render(pages_of_text("a"), deadline=time.monotonic() + 0.2, pdfium_open=slow_open)
    assert page.render_options == {}
    assert document.closed


def test_a_page_that_takes_too_long_to_draw_is_too_slow() -> None:
    class SlowPage(RecordingPage):
        def render(self, **options: object) -> Any:
            time.sleep(0.3)
            return super().render(**options)

    document = OnePageDocument(SlowPage())
    with pytest.raises(TooSlow):
        render(pages_of_text("a"), deadline=time.monotonic() + 0.2, pdfium_open=lambda d: document)
    assert document.closed
