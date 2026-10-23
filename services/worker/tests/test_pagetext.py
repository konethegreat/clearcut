# Written by Kone Tshivhinda & Claude
"""Reading a page's text, where each word is, and what fonts drew it (ADR-0016)."""

from __future__ import annotations

import io
import random
import time
from collections.abc import Callable

import pytest
from pypdf import PdfReader

from clearcut_worker import pagetext
from clearcut_worker.images import ImageTooLarge
from clearcut_worker.inspect import Encrypted, NotAPdf, TooManyPages, TooSlow, Unreadable
from clearcut_worker.pagetext import (
    Box,
    FontFact,
    PageGeometry,
    PageOutOfRange,
    PageTooLarge,
    assemble_words,
    extract_pages,
    font_facts,
)

from .conftest import make_encrypted_pdf
from .pdfs import HELVETICA, PageSpec, PdfBuilder, one_page, pages_of_text, text_at
from .pictures import claims_to_be, jpeg, noise, png, png_of

FAR = time.monotonic() + 3600


def each_unit_a_box(raw: str) -> Callable[[int], Box | None]:
    """Gives the character at UTF-16 position i the box (i, 0, i + 1, 10)."""
    return lambda i: (float(i), 0.0, float(i + 1), 10.0)


def words_of(text: str, spans: list[tuple[int, int]]) -> list[str]:
    raw = text.encode("utf-8")
    return [raw[a:b].decode("utf-8") for a, b in spans]


# ---------- putting text and words together ----------


def test_words_are_runs_of_characters_that_are_not_white_space() -> None:
    text, words = assemble_words("Hello world", each_unit_a_box("Hello world"))
    assert text == "Hello world"
    assert [(w.start, w.end) for w in words] == [(0, 5), (6, 11)]
    assert words[0].box == (0.0, 0.0, 5.0, 10.0)
    assert words[1].box == (6.0, 0.0, 11.0, 10.0)


def test_offsets_count_utf8_bytes_not_characters() -> None:
    raw = "Café au lait"
    text, words = assemble_words(raw, each_unit_a_box(raw))
    assert [(w.start, w.end) for w in words] == [(0, 5), (6, 8), (9, 13)]
    assert words_of(text, [(w.start, w.end) for w in words]) == ["Café", "au", "lait"]
    # The box of "lait" comes from its own characters (UTF-16 positions 8 to 11).
    assert words[2].box == (8.0, 0.0, 12.0, 10.0)


def test_a_character_outside_the_basic_plane_is_four_bytes_and_two_utf16_units() -> None:
    raw = "Pay \U0001f600 now"
    text, words = assemble_words(raw, each_unit_a_box(raw))
    assert words_of(text, [(w.start, w.end) for w in words]) == ["Pay", "\U0001f600", "now"]
    assert [(w.start, w.end) for w in words] == [(0, 3), (4, 8), (9, 12)]
    # "now" sits at UTF-16 positions 7 to 9: the emoji used two of the units.
    assert words[2].box == (7.0, 0.0, 10.0, 10.0)
    assert words[1].box == (4.0, 0.0, 5.0, 10.0)


def test_line_breaks_become_a_single_new_line_character() -> None:
    raw = "Total\r\n100\rdone"
    text, words = assemble_words(raw, each_unit_a_box(raw))
    assert text == "Total\n100\ndone"
    assert words_of(text, [(w.start, w.end) for w in words]) == ["Total", "100", "done"]
    assert words[1].box == (7.0, 0.0, 10.0, 10.0)


def test_control_characters_and_noncharacters_are_dropped() -> None:
    # U+FFFE and U+FFFF, the mid-block noncharacters U+FDD0 to U+FDEF, and the last two of a plane.
    raw = "a\x00b￾￿c\x0bd﷐﷯\U0001fffe"
    text, words = assemble_words(raw, each_unit_a_box(raw))
    assert text == "abcd"
    assert [(w.start, w.end) for w in words] == [(0, 4)]


def test_a_tab_is_kept_as_white_space() -> None:
    text, words = assemble_words("a\tb", each_unit_a_box("a\tb"))
    assert text == "a\tb"
    assert len(words) == 2


def test_a_word_none_of_whose_characters_has_a_box_is_left_out_of_the_words() -> None:
    boxes: dict[int, Box] = {0: (0, 0, 1, 10), 1: (1, 0, 2, 10)}
    text, words = assemble_words("ab cd", lambda i: boxes.get(i))
    assert text == "ab cd"
    assert [(w.start, w.end) for w in words] == [(0, 2)]


def test_a_box_with_no_size_is_not_a_box() -> None:
    _text, words = assemble_words("ab", lambda i: (0.0, 0.0, 0.0, 0.0) if i == 0 else (1, 0, 2, 10))
    assert words[0].box == (1.0, 0.0, 2.0, 10.0)


def test_nothing_or_only_white_space_gives_no_text_and_no_words() -> None:
    for raw in ("", "   ", "\r\n\r\n", " \t "):
        text, words = assemble_words(raw, each_unit_a_box(raw))
        assert words == []
        assert not text.strip()


def test_every_word_decodes_to_exactly_the_characters_it_covers() -> None:
    rng = random.Random(7)  # noqa: S311 - a fixed spread of awkward strings, not security
    alphabet = [
        "a",
        "Z",
        "é",
        "ß",
        "€",
        "\U0001f600",
        " ",
        " ",
        "\n",
        "\r\n",
        "\t",
        "7",
        ".",
        "\x00",
    ]
    for _ in range(2000):
        raw = "".join(rng.choice(alphabet) for _ in range(rng.randrange(0, 30)))
        text, words = assemble_words(raw, each_unit_a_box(raw))
        encoded = text.encode("utf-8")
        assert "\r" not in text
        last_end = 0
        for word in words:
            assert last_end <= word.start < word.end <= len(encoded)
            piece = encoded[word.start : word.end].decode("utf-8")
            assert piece == piece.strip()
            assert not any(c.isspace() for c in piece)
            last_end = word.end


# ---------- where a box is on the page ----------

CROP: Box = (0.0, 0.0, 200.0, 100.0)


def test_an_upright_page_measures_from_the_top_left_in_thousandths() -> None:
    geometry = PageGeometry(crop=CROP, rotation=0)
    assert (geometry.width_pt, geometry.height_pt) == (200.0, 100.0)
    # Left 20 to right 60 of 200 wide is 100 to 300; top 30 down from the top is 70 to 90 of 100.
    assert geometry.to_permille((20, 10, 60, 30)) == (100, 700, 200, 200)


def test_a_page_turned_ninety_degrees_swaps_its_sides() -> None:
    geometry = PageGeometry(crop=CROP, rotation=90)
    assert (geometry.width_pt, geometry.height_pt) == (100.0, 200.0)
    assert geometry.to_permille((20, 10, 60, 30)) == (100, 100, 200, 200)


def test_a_page_turned_half_way_is_mirrored_in_both_directions() -> None:
    geometry = PageGeometry(crop=CROP, rotation=180)
    assert (geometry.width_pt, geometry.height_pt) == (200.0, 100.0)
    assert geometry.to_permille((20, 10, 60, 30)) == (700, 100, 200, 200)


def test_a_page_turned_two_hundred_and_seventy_degrees() -> None:
    geometry = PageGeometry(crop=CROP, rotation=270)
    assert (geometry.width_pt, geometry.height_pt) == (100.0, 200.0)
    assert geometry.to_permille((20, 10, 60, 30)) == (700, 700, 200, 200)


def test_the_visible_area_is_the_crop_box_not_the_whole_sheet() -> None:
    geometry = PageGeometry(crop=(50.0, 50.0, 250.0, 150.0), rotation=0)
    assert (geometry.width_pt, geometry.height_pt) == (200.0, 100.0)
    assert geometry.to_permille((70, 60, 110, 80)) == (100, 700, 200, 200)


def test_boxes_are_rounded_outwards_so_they_never_cover_less_than_the_glyph() -> None:
    geometry = PageGeometry(crop=(0.0, 0.0, 100.0, 100.0), rotation=0)
    x, y, w, h = geometry.to_permille((10.5, 10.5, 20.25, 20.5))
    assert (x, w) == (105, 98)  # 105.0 to 202.5 becomes 105 to 203
    assert (y, h) == (795, 100)  # top 79.5 to 89.5 of 100 becomes 795 to 895


def test_a_box_beyond_the_page_is_clamped_to_it() -> None:
    geometry = PageGeometry(crop=CROP, rotation=0)
    x, y, w, h = geometry.to_permille((-50, -50, 400, 300))
    assert (x, y) == (0, 0)
    assert x + w == 1000
    assert y + h == 1000


def test_a_box_is_never_smaller_than_a_thousandth_and_never_leaves_the_page() -> None:
    geometry = PageGeometry(crop=CROP, rotation=0)
    for box in [(200.0, 100.0, 200.0, 100.0), (0.0, 0.0, 0.0, 0.0), (199.99, 0.0, 200.0, 0.01)]:
        x, y, w, h = geometry.to_permille(box)
        assert w >= 1
        assert h >= 1
        assert x >= 0
        assert x + w <= 1000
        assert y >= 0
        assert y + h <= 1000


def test_a_page_with_no_area_is_refused() -> None:
    for crop in [(0.0, 0.0, 0.0, 100.0), (0.0, 0.0, 100.0, 0.0), (10.0, 10.0, 5.0, 50.0)]:
        with pytest.raises(ValueError, match="area"):
            PageGeometry(crop=crop, rotation=0)


def test_only_quarter_turns_are_accepted() -> None:
    with pytest.raises(ValueError, match="rotation"):
        PageGeometry(crop=CROP, rotation=45)
    assert PageGeometry(crop=CROP, rotation=-90).rotation == 270
    assert PageGeometry(crop=CROP, rotation=450).rotation == 90


# ---------- the fonts a page was drawn with ----------


def facts(pdf: bytes, page: int = 0) -> tuple[FontFact, ...]:
    return font_facts(PdfReader(io.BytesIO(pdf)).pages[page])


def with_font(body: str, **objects: str | bytes) -> bytes:
    builder = PdfBuilder(
        pages=[PageSpec(content="", resources="/Font << /F1 @thefont@ >>")],
        objects={"thefont": body, **objects},
    )
    return builder.build()


def test_a_plain_standard_font_has_no_encoding_map_or_embedded_program() -> None:
    assert facts(one_page(text_at(20, 100, "x"))) == (
        FontFact(kind="type1", has_to_unicode=False, encoding="none", embedded=False),
    )


def test_the_named_encodings_are_recognised() -> None:
    for name, expected in [
        ("WinAnsiEncoding", "winansi"),
        ("MacRomanEncoding", "macroman"),
        ("StandardEncoding", "standard"),
        ("MacExpertEncoding", "other"),
    ]:
        pdf = with_font(
            f"<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /{name} >>"
        )
        assert facts(pdf) == (FontFact("type1", False, expected, False),), name


def test_an_encoding_that_redefines_characters_is_custom() -> None:
    pdf = with_font(
        "<< /Type /Font /Subtype /TrueType /BaseFont /Arial "
        "/Encoding << /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [65 /B /A] >> >>"
    )
    assert facts(pdf) == (FontFact("truetype", False, "custom", False),)


def test_a_unicode_map_is_noticed() -> None:
    pdf = with_font(
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode @map@ >>",
        map=b"<< /Length 0 >>\nstream\n\nendstream",
    )
    assert facts(pdf) == (FontFact("type1", True, "none", False),)


def test_a_composite_font_with_identity_encoding_and_no_unicode_map() -> None:
    pdf = with_font(
        "<< /Type /Font /Subtype /Type0 /BaseFont /ABCDEF+Thing /Encoding /Identity-H "
        "/DescendantFonts [@cid@] >>",
        cid="<< /Type /Font /Subtype /CIDFontType2 /BaseFont /ABCDEF+Thing "
        "/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> >>",
    )
    assert facts(pdf) == (FontFact("type0", False, "identity", False),)


def test_an_embedded_font_program_is_noticed_through_the_descendant_font() -> None:
    pdf = with_font(
        "<< /Type /Font /Subtype /Type0 /BaseFont /ABCDEF+Thing /Encoding /Identity-H "
        "/DescendantFonts [@cid@] /ToUnicode @map@ >>",
        cid="<< /Type /Font /Subtype /CIDFontType2 /BaseFont /ABCDEF+Thing "
        "/FontDescriptor @descriptor@ >>",
        descriptor="<< /Type /FontDescriptor /FontName /ABCDEF+Thing /FontFile2 @program@ >>",
        program=b"<< /Length 4 >>\nstream\nabcd\nendstream",
        map=b"<< /Length 0 >>\nstream\n\nendstream",
    )
    assert facts(pdf) == (FontFact("type0", True, "identity", True),)


def test_a_font_embedded_in_a_simple_font_is_noticed_too() -> None:
    for key in ("FontFile", "FontFile2", "FontFile3"):
        pdf = with_font(
            "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+Thing /FontDescriptor @d@ >>",
            d=f"<< /Type /FontDescriptor /FontName /ABCDEF+Thing /{key} @p@ >>",
            p=b"<< /Length 4 >>\nstream\nabcd\nendstream",
        )
        assert facts(pdf) == (FontFact("truetype", False, "none", True),), key


def test_an_embedded_cmap_makes_a_composite_font_custom_and_a_named_one_other() -> None:
    custom = with_font(
        "<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding @cmap@ /DescendantFonts [@cid@] >>",
        cmap=b"<< /Type /CMap /CMapName /Custom /Length 0 >>\nstream\n\nendstream",
        cid="<< /Type /Font /Subtype /CIDFontType2 /BaseFont /X >>",
    )
    assert facts(custom) == (FontFact("type0", False, "custom", False),)
    named = with_font(
        "<< /Type /Font /Subtype /Type0 /BaseFont /X /Encoding /UniJIS-UCS2-H "
        "/DescendantFonts [@cid@] >>",
        cid="<< /Type /Font /Subtype /CIDFontType0 /BaseFont /X >>",
    )
    assert facts(named) == (FontFact("type0", False, "other", False),)


def test_a_type_3_font_draws_its_own_glyphs() -> None:
    pdf = with_font(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 10 10] /FontMatrix [0.1 0 0 0.1 0 0] "
        "/CharProcs << /a @glyph@ >> /FirstChar 97 /LastChar 97 /Widths [10] "
        "/Encoding << /Type /Encoding /Differences [97 /a] >> >>",
        glyph=b"<< /Length 18 >>\nstream\n10 0 0 0 10 10 d1\nendstream",
    )
    assert facts(pdf) == (FontFact("type3", False, "custom", True),)


def test_an_unknown_kind_of_font_is_other() -> None:
    assert facts(with_font("<< /Type /Font /Subtype /Something /BaseFont /X >>")) == (
        FontFact("other", False, "none", False),
    )


def test_the_same_font_used_twice_is_listed_once_and_the_list_is_in_a_fixed_order() -> None:
    builder = PdfBuilder(
        pages=[PageSpec(content="", resources="/Font << /A @h@ /B @h2@ /C @t@ >>")],
        objects={
            "h": HELVETICA,
            "h2": HELVETICA,
            "t": "<< /Type /Font /Subtype /TrueType /BaseFont /Arial >>",
        },
    )
    assert facts(builder.build()) == (
        FontFact("truetype", False, "none", False),
        FontFact("type1", False, "none", False),
    )


def test_fonts_used_only_inside_a_form_are_found() -> None:
    builder = PdfBuilder(
        pages=[PageSpec(content="/Fm1 Do", resources="/XObject << /Fm1 @form@ >>")],
        objects={
            "form": b"<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] "
            b"/Resources << /Font << /F9 @deep@ >> >> /Length 0 >>\nstream\n\nendstream",
            "deep": "<< /Type /Font /Subtype /TrueType /BaseFont /Deep >>",
        },
    )
    assert facts(builder.build()) == (FontFact("truetype", False, "none", False),)


def test_a_form_that_contains_itself_does_not_loop_for_ever() -> None:
    builder = PdfBuilder(
        pages=[PageSpec(content="/Fm1 Do", resources="/XObject << /Fm1 @form@ >>")],
        objects={
            "form": b"<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] "
            b"/Resources << /XObject << /Fm1 @form@ >> /Font << /F1 @helvetica@ >> >> "
            b"/Length 0 >>\nstream\n\nendstream",
        },
    )
    assert facts(builder.build()) == (FontFact("type1", False, "none", False),)


def test_a_page_with_no_fonts_or_a_broken_font_entry_has_none_to_report() -> None:
    assert facts(PdfBuilder(pages=[PageSpec(content="", resources="")]).build()) == ()
    broken = PdfBuilder(
        pages=[PageSpec(content="", resources="/Font << /F1 42 /F2 (text) /F3 null >>")]
    ).build()
    assert facts(broken) == ()


def test_a_page_with_a_great_many_fonts_is_cut_off_rather_than_followed_for_ever() -> None:
    fonts = "".join(f"/F{i} @h@ " for i in range(1)) + "".join(
        f"/G{i} << /Type /Font /Subtype /TrueType /BaseFont /N{i} /Encoding /WinAnsiEncoding >> "
        for i in range(400)
    )
    builder = PdfBuilder(
        pages=[PageSpec(content="", resources=f"/Font << {fonts} >>")], objects={"h": HELVETICA}
    )
    result = facts(builder.build())
    assert 1 <= len(result) <= 64


# ---------- reading a whole document ----------


def read(
    pdf: bytes, first: int = 1, last: int | None = None, *, words: bool = False, **options: object
) -> list:  # type: ignore[type-arg]
    end = len(PdfReader(io.BytesIO(pdf)).pages) if last is None else last
    return extract_pages(
        pdf,
        first,
        end,
        words=words,
        max_pages=options.pop("max_pages", 50),  # type: ignore[arg-type]
        deadline=options.pop("deadline", FAR),  # type: ignore[arg-type]
        **options,  # type: ignore[arg-type]
    )


def test_each_pages_text_comes_back_with_its_number_and_size() -> None:
    pdf = pages_of_text("Invoice INV-1", None, "Total 100.00")
    pages = read(pdf)
    assert [p.page for p in pages] == [1, 2, 3]
    assert "Invoice INV-1" in pages[0].text
    assert pages[0].source == "text_layer"
    assert (pages[0].width_pt, pages[0].height_pt) == (200.0, 200.0)
    assert "Total 100.00" in pages[2].text
    assert pages[0].words is None


def test_a_blank_page_has_no_text_and_says_so() -> None:
    pages = read(pages_of_text(None), words=True)
    assert pages[0].source == "none"
    assert pages[0].text == ""
    assert pages[0].words == ()
    assert pages[0].fonts == ()


def test_a_page_of_only_white_space_counts_as_having_no_text() -> None:
    pages = read(pages_of_text("   "))
    assert pages[0].source == "none"
    assert pages[0].text == ""


def test_a_range_of_pages_can_be_asked_for() -> None:
    pdf = pages_of_text("one", "two", "three", "four")
    assert [p.page for p in read(pdf, 2, 3)] == [2, 3]
    assert "three" in read(pdf, 3, 3)[0].text


def test_the_fonts_each_page_used_are_reported_with_it() -> None:
    pages = read(pages_of_text("hello"))
    assert pages[0].fonts == (FontFact("type1", False, "none", False),)


def test_words_come_with_offsets_into_the_text_and_boxes_on_the_page() -> None:
    pdf = one_page(text_at(20, 100, "Invoice INV-1", size=12))
    page = read(pdf, words=True)[0]
    assert page.words is not None
    encoded = page.text.encode("utf-8")
    spoken = [encoded[w.start : w.end].decode("utf-8") for w in page.words]
    assert spoken == ["Invoice", "INV-1"]
    first, second = page.words
    # Drawn 20 points from the left of a 200 point page: about 100 thousandths.
    assert 90 <= first.x <= 110
    # A 12 point line whose baseline is 100 points up a 200 point page: just above the middle.
    assert 380 <= first.y <= 480
    # The box is the whole line (ascender to descender), not just the ink, so that a block
    # painted over it hides the descenders too: about 13 points of a 200 point page.
    assert 60 <= first.h <= 120
    assert second.x > first.x + first.w - 5
    for word in page.words:
        assert word.x >= 0
        assert word.x + word.w <= 1000
        assert word.y + word.h <= 1000


def test_accented_text_has_utf8_offsets_that_match() -> None:
    pdf = PdfBuilder(
        pages=[PageSpec(content="BT /F1 12 Tf 20 100 Td (Caf\\351 au lait) Tj ET\n")],
        objects={
            "helvetica": "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica "
            "/Encoding /WinAnsiEncoding >>"
        },
    ).build()
    page = read(pdf, words=True)[0]
    assert "Café au lait" in page.text
    assert page.words is not None
    encoded = page.text.encode("utf-8")
    spoken = [encoded[w.start : w.end].decode("utf-8") for w in page.words]
    assert spoken == ["Café", "au", "lait"]


def test_a_turned_page_reports_its_displayed_size_and_boxes_in_that_orientation() -> None:
    pdf = one_page(text_at(20, 50, "Rotated"), media_box="[0 0 200 100]", rotate=90)
    page = read(pdf, words=True)[0]
    assert (page.width_pt, page.height_pt) == (100.0, 200.0)
    assert page.words is not None
    word = page.words[0]
    # Text at (20, 50) on the unturned sheet lands about half way across, a tenth of the way down.
    assert 440 <= word.x <= 520
    assert 80 <= word.y <= 130


def test_a_cropped_page_reports_the_cropped_size_and_positions() -> None:
    pdf = one_page(
        text_at(60, 100, "Cropped"), media_box="[0 0 200 200]", crop_box="[50 50 150 150]"
    )
    page = read(pdf, words=True)[0]
    assert (page.width_pt, page.height_pt) == (100.0, 100.0)
    assert page.words is not None
    word = page.words[0]
    assert 90 <= word.x <= 115
    assert 380 <= word.y <= 440


def test_a_page_range_outside_the_document_says_how_many_pages_there_are() -> None:
    pdf = pages_of_text("a", "b")
    with pytest.raises(PageOutOfRange) as caught:
        read(pdf, 2, 3)
    assert caught.value.pages == 2


def test_a_backwards_or_zero_range_is_a_programming_error() -> None:
    pdf = pages_of_text("a", "b")
    for first, last in [(0, 1), (2, 1), (-1, 1)]:
        with pytest.raises(ValueError, match="range"):
            extract_pages(pdf, first, last, words=False, max_pages=50, deadline=FAR)


def test_what_cannot_be_read_is_refused_in_a_named_way() -> None:
    with pytest.raises(NotAPdf):
        read(b"<html>nope</html>", 1, 1)
    with pytest.raises(Encrypted):
        read(make_encrypted_pdf(), 1, 1)
    with pytest.raises(Unreadable):
        read(b"%PDF-1.4\nSECRET garbage", 1, 1)
    with pytest.raises(TooManyPages):
        read(pages_of_text("a", "b", "c"), 1, 1, max_pages=2)


def test_a_deadline_that_has_passed_stops_the_reading() -> None:
    with pytest.raises(TooSlow):
        read(pages_of_text("a", "b"), 1, 2, deadline=time.monotonic() - 1)


def test_a_page_with_more_text_than_allowed_is_refused_not_cut() -> None:
    with pytest.raises(PageTooLarge):
        read(pages_of_text("x" * 200), 1, 1, max_page_text_bytes=100)
    assert read(pages_of_text("x" * 200), 1, 1, max_page_text_bytes=1000)[0].text


def test_a_page_with_more_words_than_allowed_is_refused_not_cut() -> None:
    pdf = one_page(text_at(10, 100, "a b c d e f g h", size=6))
    with pytest.raises(PageTooLarge):
        read(pdf, 1, 1, words=True, max_words_per_page=5)


class FakePdfiumPage:
    """A page that reads perfectly well, so only what a test changes can make it fail."""

    def __init__(
        self,
        text: str = "hello",
        box: Callable[[int], tuple[float, float, float, float]] | None = None,
        pause: float = 0.0,
    ) -> None:
        self.text = text
        self.box = box
        self.pause = pause
        self.closes = 0

    def get_cropbox(self) -> tuple[float, float, float, float]:
        return (0.0, 0.0, 200.0, 200.0)

    def get_rotation(self) -> int:
        return 0

    def get_textpage(self) -> FakePdfiumPage:
        return self

    def get_text_range(self) -> str:
        time.sleep(self.pause)
        return self.text

    def get_charbox(self, index: int, loose: bool = False) -> tuple[float, float, float, float]:
        if self.box is not None:
            return self.box(index)
        return (float(index), 0.0, float(index + 1), 10.0)

    def close(self) -> None:
        self.closes += 1


class FakePdfiumDocument:
    """A PDF engine that may disagree with pypdf about how many pages there are."""

    def __init__(self, length: int, page: FakePdfiumPage | None = None) -> None:
        self.length = length
        self.page = page
        self.closed = False

    def __len__(self) -> int:
        return self.length

    def __getitem__(self, index: int) -> FakePdfiumPage:
        return self.page if self.page is not None else FakePdfiumPage()

    def close(self) -> None:
        self.closed = True


def read_fake(page: FakePdfiumPage, *, words: bool = False, **options: object) -> list:  # type: ignore[type-arg]
    document = FakePdfiumDocument(1, page)
    return read(pages_of_text("a"), 1, 1, words=words, pdfium_open=lambda data: document, **options)


def test_two_parsers_that_disagree_about_the_page_count_mean_an_unreadable_document() -> None:
    fake = FakePdfiumDocument(5)
    with pytest.raises(Unreadable):
        read(pages_of_text("a", "b"), 1, 2, pdfium_open=lambda data: fake)
    assert fake.closed, "the engine's document must be closed even when it is refused"


def test_the_same_fake_engine_with_the_right_count_reads_fine() -> None:
    # Guards the test above: it fails because of the count, not because the fake is broken.
    fake = FakePdfiumDocument(2)
    pages = read(pages_of_text("a", "b"), 1, 2, pdfium_open=lambda data: fake)
    assert [p.text for p in pages] == ["hello", "hello"]
    assert fake.closed


def test_exactly_at_the_text_and_word_limits_is_allowed() -> None:
    pdf = one_page(text_at(10, 100, "a b c d e f g h", size=6))
    page = read(pdf, 1, 1, words=True, max_words_per_page=8)[0]
    assert page.words is not None
    assert len(page.words) == 8
    exact = len(page.text.encode("utf-8"))
    assert read(pdf, 1, 1, max_page_text_bytes=exact)[0].text == page.text
    with pytest.raises(PageTooLarge):
        read(pdf, 1, 1, max_page_text_bytes=exact - 1)
    with pytest.raises(PageTooLarge):
        read(pdf, 1, 1, words=True, max_words_per_page=7)


def test_an_open_ended_range_reads_to_the_end_but_at_most_fifty_pages() -> None:
    pdf = pages_of_text(*["x"] * 52)
    tail = extract_pages(pdf, 49, None, words=False, max_pages=500, deadline=FAR)
    assert [p.page for p in tail] == [49, 50, 51, 52]
    head = extract_pages(pdf, 1, None, words=False, max_pages=500, deadline=FAR)
    assert len(head) == 50
    with pytest.raises(PageOutOfRange) as caught:
        extract_pages(pdf, 53, None, words=False, max_pages=500, deadline=FAR)
    assert caught.value.pages == 52


def test_an_engine_that_cannot_open_the_document_makes_it_unreadable_without_a_message() -> None:
    def refuse(data: bytes) -> object:
        raise RuntimeError("SECRET internals about " + data[:4].decode("latin-1"))

    with pytest.raises(Unreadable) as caught:
        read(pages_of_text("a"), 1, 1, pdfium_open=refuse)
    assert "SECRET" not in str(caught.value)


def test_the_engines_pages_are_closed_after_reading_and_after_a_refusal() -> None:
    page = FakePdfiumPage()
    read_fake(page)
    assert page.closes == 2  # the text page and the page
    big = FakePdfiumPage(text="x" * 300)
    with pytest.raises(PageTooLarge):
        read_fake(big, max_page_text_bytes=100)
    assert big.closes == 2


def test_a_page_whose_raw_text_is_huge_is_refused_even_if_most_of_it_would_be_dropped() -> None:
    with pytest.raises(PageTooLarge):
        read_fake(FakePdfiumPage(text="\x01" * 300), max_page_text_bytes=100)


def test_the_text_limit_counts_bytes_not_characters() -> None:
    # 60 characters, 120 bytes.
    with pytest.raises(PageTooLarge):
        read_fake(FakePdfiumPage(text="é" * 60), max_page_text_bytes=100)
    assert read_fake(FakePdfiumPage(text="é" * 40), max_page_text_bytes=100)[0].text


def test_a_character_the_engine_cannot_measure_leaves_its_word_out_not_the_page_unreadable() -> (
    None
):
    def refuse(index: int) -> tuple[float, float, float, float]:
        raise RuntimeError("no box")

    page = read_fake(FakePdfiumPage(text="ab cd", box=refuse), words=True)[0]
    assert page.text == "ab cd"
    assert page.words == ()


def test_the_deadline_is_checked_before_the_engine_is_even_opened() -> None:
    opened: list[bytes] = []

    def opener(data: bytes) -> FakePdfiumDocument:
        opened.append(data)
        return FakePdfiumDocument(1)

    with pytest.raises(TooSlow):
        read(pages_of_text("a"), 1, 1, deadline=time.monotonic() - 1, pdfium_open=opener)
    assert opened == []


def test_the_deadline_is_checked_between_pages() -> None:
    page = FakePdfiumPage(pause=0.4)
    document = FakePdfiumDocument(2, page)
    with pytest.raises(TooSlow):
        read(
            pages_of_text("a", "b"),
            1,
            2,
            deadline=time.monotonic() + 0.3,
            pdfium_open=lambda data: document,
        )
    assert document.closed


def test_a_pdf_with_a_damaged_cross_reference_table_is_still_read() -> None:
    pdf = pages_of_text("Invoice INV-1")
    damaged = pdf.replace(b"startxref\n", b"startxref\n9", 1)
    assert damaged != pdf
    assert "Invoice INV-1" in read(damaged)[0].text


# ---------- the same details, one level down ----------


def test_a_lone_line_feed_is_kept_and_ends_a_word() -> None:
    text, words = assemble_words("ab\ncd", each_unit_a_box("ab\ncd"))
    assert text == "ab\ncd"
    assert [(w.start, w.end) for w in words] == [(0, 2), (3, 5)]


def test_a_word_box_covers_the_tallest_and_lowest_of_its_characters() -> None:
    boxes: dict[int, Box] = {0: (0, 2, 1, 6), 1: (1, 4, 2, 9)}
    _text, words = assemble_words("ab", lambda i: boxes[i])
    assert words[0].box == (0.0, 2.0, 2.0, 9.0)


def test_a_dropped_character_still_uses_up_its_place_in_the_engines_numbering() -> None:
    boxes: dict[int, Box] = {0: (0, 0, 1, 10), 1: (50, 0, 51, 10), 2: (7, 0, 8, 10)}
    text, words = assemble_words("a\x00b", lambda i: boxes[i])
    assert text == "ab"
    # Box 2 belongs to "b"; box 1 (at x=50) belonged to the dropped character.
    assert words[0].box == (0.0, 0.0, 8.0, 10.0)


def test_a_box_with_width_but_no_height_is_not_a_box() -> None:
    boxes: dict[int, Box] = {0: (0, 5, 4, 5), 1: (4, 0, 8, 10)}
    _text, words = assemble_words("ab", lambda i: boxes[i])
    assert words[0].box == (4.0, 0.0, 8.0, 10.0)
    flat: dict[int, Box] = {0: (0, 5, 4, 5)}
    assert assemble_words("a", lambda i: flat[i])[1] == []


def test_a_box_edge_is_rounded_down_and_up_not_to_the_nearest_thousandth() -> None:
    geometry = PageGeometry(crop=(0.0, 0.0, 1000.0, 1000.0), rotation=0)
    x, _y, w, _h = geometry.to_permille((1.5, 10.0, 3.5, 20.0))
    assert (x, w) == (1, 3)


def test_a_named_base_encoding_inside_an_encoding_dictionary_is_recognised() -> None:
    pdf = with_font(
        "<< /Type /Font /Subtype /TrueType /BaseFont /Arial "
        "/Encoding << /Type /Encoding /BaseEncoding /MacRomanEncoding >> >>"
    )
    assert facts(pdf) == (FontFact("truetype", False, "macroman", False),)


def test_fonts_with_the_same_kind_are_listed_in_a_fixed_order_by_their_other_facts() -> None:
    builder = PdfBuilder(
        pages=[PageSpec(content="", resources="/Font << /A @a@ /B @b@ /C @c@ /D @d@ >>")],
        objects={
            name: f"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica {enc} >>"
            for name, enc in [
                ("a", "/Encoding /WinAnsiEncoding"),
                ("b", "/Encoding /StandardEncoding"),
                ("c", "/Encoding /MacRomanEncoding"),
                ("d", ""),
            ]
        },
    )
    assert [f.encoding for f in facts(builder.build())] == [
        "macroman",
        "none",
        "standard",
        "winansi",
    ]


def distinct_fonts(count: int) -> str:
    encodings = ["/WinAnsiEncoding", "/MacRomanEncoding", "/StandardEncoding"]
    return " ".join(
        f"/F{i} << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding {encodings[i]} >>"
        for i in range(count)
    )


def test_the_number_of_fonts_reported_for_a_page_is_capped(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(pagetext, "_MAX_FONTS_PER_PAGE", 2)
    pdf = PdfBuilder(
        pages=[PageSpec(content="", resources=f"/Font << {distinct_fonts(3)} >>")]
    ).build()
    assert len(facts(pdf)) == 2


def test_the_number_of_font_entries_looked_at_is_capped(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(pagetext, "_MAX_FONT_DICTS_VISITED", 2)
    pdf = PdfBuilder(
        pages=[PageSpec(content="", resources=f"/Font << {distinct_fonts(3)} >>")]
    ).build()
    assert [f.encoding for f in facts(pdf)] == ["macroman", "winansi"]


def chain_of_forms(depth: int) -> PdfBuilder:
    objects: dict[str, str | bytes] = {}
    for level in range(depth):
        inner = (
            f"/XObject << /Fm @form{level + 1}@ >>"
            if level + 1 < depth
            else "/Font << /F9 @deep@ >>"
        )
        objects[f"form{level}"] = (
            "<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] "
            f"/Resources << {inner} >> /Length 0 >>\nstream\n\nendstream"
        ).encode()
    objects["deep"] = "<< /Type /Font /Subtype /TrueType /BaseFont /Deep >>"
    return PdfBuilder(
        pages=[PageSpec(content="", resources="/XObject << /Fm @form0@ >>")], objects=objects
    )


def test_forms_inside_forms_are_followed_only_so_deep(monkeypatch: pytest.MonkeyPatch) -> None:
    builder = chain_of_forms(3)
    assert facts(builder.build()) == (FontFact("truetype", False, "none", False),)
    monkeypatch.setattr(pagetext, "_MAX_FORM_DEPTH", 2)
    assert facts(builder.build()) == ()


def test_the_number_of_forms_looked_at_is_capped(monkeypatch: pytest.MonkeyPatch) -> None:
    objects: dict[str, str | bytes] = {}
    for n, enc in enumerate(["WinAnsiEncoding", "MacRomanEncoding"]):
        objects[f"form{n}"] = (
            "<< /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /Font << /F1 "
            f"<< /Type /Font /Subtype /Type1 /BaseFont /H /Encoding /{enc} >> >> >> /Length 0 >>"
            "\nstream\n\nendstream"
        ).encode()
    pdf = PdfBuilder(
        pages=[PageSpec(content="", resources="/XObject << /A @form0@ /B @form1@ >>")],
        objects=objects,
    ).build()
    assert len(facts(pdf)) == 2
    monkeypatch.setattr(pagetext, "_MAX_FORMS_VISITED", 1)
    assert len(facts(pdf)) == 1


# ---------- a picture is a one-page document ----------


def read_picture(data: bytes, first: int = 1, last: int | None = 1, *, words: bool = False) -> list:  # type: ignore[type-arg]
    return extract_pages(data, first, last, words=words, max_pages=50, deadline=FAR)


def test_a_picture_has_one_page_with_no_text_and_its_size_in_pixels() -> None:
    [page] = read_picture(png(300, 200), words=True)
    assert page.page == 1
    assert page.source == "none"
    assert page.text == ""
    assert (page.width_pt, page.height_pt) == (300.0, 200.0)
    assert page.fonts == ()
    assert page.words == ()


def test_a_picture_has_no_words_unless_asked_and_then_an_empty_list() -> None:
    assert read_picture(png())[0].words is None
    assert read_picture(png(), words=True)[0].words == ()


def test_a_photo_is_measured_upright() -> None:
    [page] = read_picture(jpeg(200, 100, orientation=6))
    assert (page.width_pt, page.height_pt) == (100.0, 200.0)


def test_only_page_one_of_a_picture_exists() -> None:
    assert len(read_picture(png(), 1, None)) == 1
    for first, last in [(2, 2), (2, None), (1, 2)]:
        with pytest.raises(PageOutOfRange) as caught:
            read_picture(png(), first, last)
        assert caught.value.pages == 1


def test_a_damaged_or_enormous_picture_is_refused() -> None:
    with pytest.raises(Unreadable):
        read_picture(png_of(noise(40, 40))[:3000])
    with pytest.raises(ImageTooLarge):
        read_picture(claims_to_be(30_000, 30_000))


def test_a_picture_gets_the_deadline_check() -> None:
    with pytest.raises(TooSlow):
        extract_pages(png(), 1, 1, words=False, max_pages=50, deadline=time.monotonic() - 1)
