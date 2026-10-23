# Written by Kone Tshivhinda & Claude
"""Reading text in a child process, and not believing what the child says (ADR-0016)."""

from __future__ import annotations

import json
import time
from typing import Any

import pytest

from clearcut_worker.inspect import Encrypted, NotAPdf, TooManyPages, TooSlow, Unreadable
from clearcut_worker.isolation import (
    DEFAULT_TEXT_ANSWER_BYTES,
    decode_text_answer,
    text_isolated,
)
from clearcut_worker.pagetext import (
    FontFact,
    PageContent,
    PageOutOfRange,
    PageTooLarge,
    Word,
    extract_pages,
)

from . import children
from .conftest import make_encrypted_pdf
from .pdfs import one_page, pages_of_text, text_at
from .test_isolation import no_children_left

FAR = time.monotonic() + 3600


def read(pdf: bytes, first: int = 1, last: int | None = 1, **options: Any) -> list[PageContent]:
    settings: dict[str, Any] = {"words": False, "max_pages": 50, "timeout_seconds": 30.0}
    settings.update(options)
    return text_isolated(pdf, first, last, **settings)


# ---------- the whole trip ----------


def test_the_answer_from_the_child_equals_reading_in_this_process() -> None:
    pdf = pages_of_text("Invoice INV-1", None, "Total 100.00")
    inside = extract_pages(pdf, 1, 3, words=True, max_pages=50, deadline=FAR)
    assert read(pdf, 1, 3, words=True) == inside
    assert no_children_left()


def test_without_words_none_are_sent_and_with_words_they_are() -> None:
    pdf = one_page(text_at(20, 100, "Invoice INV-1"))
    assert read(pdf)[0].words is None
    words = read(pdf, words=True)[0].words
    assert words is not None
    assert len(words) == 2


def test_leaving_out_the_last_page_reads_to_the_end_of_the_document() -> None:
    pdf = pages_of_text("one", "two", "three")
    assert [p.page for p in read(pdf, 2, None)] == [2, 3]


def test_an_open_ended_range_is_cut_at_fifty_pages() -> None:
    pdf = pages_of_text(*["x"] * 52)
    assert [p.page for p in read(pdf, 1, None, max_pages=500)] == list(range(1, 51))
    assert [p.page for p in read(pdf, 51, None, max_pages=500)] == [51, 52]


def test_every_kind_of_refusal_survives_the_trip_back_from_the_child() -> None:
    with pytest.raises(NotAPdf):
        read(b"<html>nope</html>")
    with pytest.raises(Encrypted):
        read(make_encrypted_pdf())
    with pytest.raises(Unreadable):
        read(b"%PDF-1.4\nSECRET garbage")
    with pytest.raises(TooManyPages) as many:
        read(pages_of_text("a", "b", "c"), max_pages=2)
    assert many.value.max_pages == 2
    with pytest.raises(PageOutOfRange) as out:
        read(pages_of_text("a", "b"), 2, 3)
    assert out.value.pages == 2
    with pytest.raises(TooSlow):
        read(pages_of_text("a"), timeout_seconds=0.0)


def test_a_page_with_too_much_text_is_refused_through_the_child() -> None:
    pdf = pages_of_text("x" * 200)
    with pytest.raises(PageTooLarge):
        read(pdf, max_page_text_bytes=100)


# ---------- hostile children ----------


def test_a_parser_stuck_for_ever_is_killed_at_the_hard_limit() -> None:
    started = time.monotonic()
    with pytest.raises(TooSlow):
        read(
            pages_of_text("a"),
            timeout_seconds=0.5,
            grace_seconds=0.5,
            target=children.text_hang,
        )
    assert time.monotonic() - started < 15
    assert no_children_left()


def test_a_parser_that_dies_hard_is_an_unreadable_document() -> None:
    with pytest.raises(Unreadable):
        read(pages_of_text("a"), target=children.text_crash)
    assert no_children_left()


def echo(answer: bytes, first: int = 1, last: int | None = 1, **options: Any) -> list[PageContent]:
    return read(answer, first, last, target=children.text_echo, **options)


def test_an_answer_over_the_size_cap_is_unreadable() -> None:
    with pytest.raises(Unreadable):
        echo(b"x" * 2048, max_answer_bytes=1024)
    assert DEFAULT_TEXT_ANSWER_BYTES >= 16 * 1024 * 1024


# ---------- checking an answer field by field ----------


def page(number: int = 1, **overrides: Any) -> dict[str, Any]:
    base: dict[str, Any] = {
        "page": number,
        "source": "text_layer",
        "text": "Café 100",
        "width_pt": 595.0,
        "height_pt": 842.5,
        "fonts": [
            {"kind": "type1", "has_to_unicode": False, "encoding": "winansi", "embedded": False}
        ],
    }
    base.update(overrides)
    return base


def ok(*pages: dict[str, Any]) -> bytes:
    return json.dumps(["ok", list(pages)], ensure_ascii=False).encode("utf-8")


def decode(
    raw: bytes, first: int = 1, last: int | None = 1, words: bool = False
) -> list[PageContent]:
    return decode_text_answer(raw, first, last, words, max_pages=50)


def test_a_well_formed_answer_is_accepted_and_becomes_page_content() -> None:
    [content] = decode(ok(page()))
    assert content == PageContent(
        page=1,
        source="text_layer",
        text="Café 100",
        width_pt=595.0,
        height_pt=842.5,
        fonts=(FontFact("type1", False, "winansi", False),),
        words=None,
    )


def test_words_are_accepted_when_asked_for_and_must_be_sound() -> None:
    words = [{"start": 0, "end": 5, "x": 100, "y": 200, "w": 300, "h": 40}]
    [content] = decode(ok(page(words=words)), words=True)
    assert content.words == (Word(0, 5, 100, 200, 300, 40),)


@pytest.mark.parametrize(
    "answer",
    [
        b"",
        b"not json",
        b"null",
        b"[]",
        b'["ok"]',
        b'["ok", {}]',
        b'["ok", [], "extra"]',
        b'["ok", [1]]',
        b'["maybe", []]',
        b'["error"]',
        b'["error", 5]',
        b'["error", "SomethingNew"]',
        b'["error", "PageOutOfRange"]',
        b'["error", "PageOutOfRange", -1]',
        b'["error", "PageOutOfRange", true]',
        b'["error", "TooManyPages", 3]',
        b"\xff\xfe broken utf-8",
        b'["ok", [{"page": NaN}]]',
    ],
)
def test_nonsense_is_unreadable(answer: bytes) -> None:
    with pytest.raises(Unreadable):
        decode(answer)


def test_the_error_names_come_back_as_themselves() -> None:
    for name, expected in [
        ("NotAPdf", NotAPdf),
        ("Encrypted", Encrypted),
        ("Unreadable", Unreadable),
        ("TooSlow", TooSlow),
        ("PageTooLarge", PageTooLarge),
        ("TooManyPages", TooManyPages),
    ]:
        with pytest.raises(expected):
            decode(json.dumps(["error", name]).encode())
    with pytest.raises(PageOutOfRange) as caught:
        decode(json.dumps(["error", "PageOutOfRange", 7]).encode())
    assert caught.value.pages == 7


@pytest.mark.parametrize(
    "bad_pages",
    [
        [],  # asked for one page
        [page(2)],  # the wrong page
        [page(1), page(2)],  # too many
    ],
)
def test_the_pages_must_be_exactly_the_ones_asked_for(bad_pages: list[dict[str, Any]]) -> None:
    with pytest.raises(Unreadable):
        decode(ok(*bad_pages), first=1, last=1)


def test_an_open_ended_range_accepts_consecutive_pages_up_to_fifty_and_no_more() -> None:
    assert len(decode(ok(page(2), page(3)), first=2, last=None)) == 2
    with pytest.raises(Unreadable):
        decode(ok(page(2), page(4)), first=2, last=None)
    with pytest.raises(Unreadable):
        decode(ok(), first=2, last=None)
    with pytest.raises(Unreadable):
        decode(ok(*[page(n) for n in range(1, 52)]), first=1, last=None)


@pytest.mark.parametrize(
    "overrides",
    [
        {"source": "ocr"},
        {"source": 5},
        {"text": 5},
        {"text": "line\r\nbreak"},
        {"text": "bell\x07"},
        {"text": "x" * 20},  # over a text cap of 10 bytes (below)
        {"source": "none", "text": "not empty"},
        {"source": "text_layer", "text": ""},
        {"source": "text_layer", "text": "   "},
        {"width_pt": 0},
        {"width_pt": -3.5},
        {"width_pt": 1e9},
        {"height_pt": "tall"},
        {"width_pt": True},
        {"fonts": "none"},
        {"fonts": [5]},
        {
            "fonts": [
                {"kind": "type9", "has_to_unicode": False, "encoding": "none", "embedded": False}
            ]
        },
        {"fonts": [{"kind": "type1", "has_to_unicode": 0, "encoding": "none", "embedded": False}]},
        {
            "fonts": [
                {"kind": "type1", "has_to_unicode": False, "encoding": "rot13", "embedded": False}
            ]
        },
        {"fonts": [{"kind": "type1", "has_to_unicode": False, "encoding": "none"}]},
        {
            "fonts": [
                {
                    "kind": "type1",
                    "has_to_unicode": False,
                    "encoding": "none",
                    "embedded": False,
                    "extra": 1,
                }
            ]
        },
        {"extra": "key"},
    ],
)
def test_a_page_entry_that_is_not_what_was_promised_is_unreadable(
    overrides: dict[str, Any],
) -> None:
    with pytest.raises(Unreadable):
        decode_text_answer(ok(page(**overrides)), 1, 1, False, max_pages=50, max_page_text_bytes=10)


def test_a_page_with_no_text_is_reported_with_no_fonts() -> None:
    [content] = decode(ok(page(source="none", text="", fonts=[])))
    assert content.source == "none"
    assert content.fonts == ()
    with pytest.raises(Unreadable):
        decode(
            ok(
                page(
                    source="none",
                    text="",
                    fonts=[
                        {
                            "kind": "type1",
                            "has_to_unicode": False,
                            "encoding": "none",
                            "embedded": False,
                        }
                    ],
                )
            )
        )


@pytest.mark.parametrize(
    "words",
    [
        [{"start": 0, "end": 0, "x": 0, "y": 0, "w": 1, "h": 1}],  # empty
        [{"start": 5, "end": 3, "x": 0, "y": 0, "w": 1, "h": 1}],  # backwards
        [{"start": 0, "end": 99, "x": 0, "y": 0, "w": 1, "h": 1}],  # past the end of the text
        [{"start": -1, "end": 3, "x": 0, "y": 0, "w": 1, "h": 1}],
        [
            {"start": 0, "end": 4, "x": 0, "y": 0, "w": 1, "h": 1}
        ],  # ends inside the 'é' (bytes 3 to 5)
        [{"start": 4, "end": 6, "x": 0, "y": 0, "w": 1, "h": 1}],  # starts inside the 'é'
        [
            {"start": 0, "end": 5, "x": 0, "y": 0, "w": 1, "h": 1},
            {"start": 3, "end": 8, "x": 0, "y": 0, "w": 1, "h": 1},
        ],
        [
            {"start": 6, "end": 9, "x": 0, "y": 0, "w": 1, "h": 1},
            {"start": 0, "end": 5, "x": 0, "y": 0, "w": 1, "h": 1},
        ],
        [{"start": 0, "end": 9, "x": 0, "y": 0, "w": 1, "h": 1}],  # spans a space
        [{"start": 0, "end": 5, "x": -1, "y": 0, "w": 1, "h": 1}],
        [{"start": 0, "end": 5, "x": 0, "y": 1001, "w": 1, "h": 1}],
        [{"start": 0, "end": 5, "x": 999, "y": 0, "w": 2, "h": 1}],  # leaves the page
        [{"start": 0, "end": 5, "x": 0, "y": 999, "w": 1, "h": 2}],
        [{"start": 0, "end": 5, "x": 0, "y": 0, "w": 0, "h": 1}],
        [{"start": 0, "end": 5, "x": 0, "y": 0, "w": 1, "h": 0}],
        [{"start": 0, "end": 5, "x": 0.5, "y": 0, "w": 1, "h": 1}],
        [{"start": 0, "end": 5, "x": True, "y": 0, "w": 1, "h": 1}],
        [{"start": 0, "end": 5, "x": 0, "y": 0, "w": 1}],
        [{"start": 0, "end": 5, "x": 0, "y": 0, "w": 1, "h": 1, "t": "x"}],
        ["word"],
        "words",
        5,
        None,
        {"start": 0},
    ],
)
def test_a_word_that_does_not_fit_the_text_or_the_page_is_unreadable(words: Any) -> None:
    with pytest.raises(Unreadable):
        decode(ok(page(words=words)), words=True)


def test_words_that_were_not_asked_for_must_not_be_there_and_asked_for_must_be() -> None:
    word = {"start": 0, "end": 5, "x": 0, "y": 0, "w": 1, "h": 1}
    with pytest.raises(Unreadable):
        decode(ok(page(words=[word])), words=False)
    with pytest.raises(Unreadable):
        decode(ok(page()), words=True)


def test_a_flood_of_words_is_refused() -> None:
    first = {"start": 0, "end": 5, "x": 0, "y": 0, "w": 1, "h": 1}
    second = {"start": 6, "end": 9, "x": 5, "y": 0, "w": 1, "h": 1}
    assert len(decode(ok(page(words=[first, second])), words=True)[0].words or ()) == 2
    with pytest.raises(Unreadable):
        decode_text_answer(
            ok(page(words=[first, second])),
            1,
            1,
            True,
            max_pages=50,
            max_words_per_page=1,
        )


def test_an_answer_nested_too_deeply_to_parse_is_unreadable() -> None:
    with pytest.raises(Unreadable):
        decode(b"[" * 100_000 + b"]" * 100_000)


@pytest.mark.parametrize("answer", [b'{"a": 1}', b"5", b'"ok"', b"true"])
def test_an_answer_that_is_not_a_list_is_unreadable_whatever_it_is(answer: bytes) -> None:
    with pytest.raises(Unreadable):
        decode(answer)


def test_an_error_with_more_in_it_than_a_name_is_not_believed() -> None:
    with pytest.raises(Unreadable):
        decode(b'["error", "TooSlow", "and more"]')
    with pytest.raises(Unreadable):
        decode(b'["error", "TooManyPages", 3]')


def test_a_page_number_must_be_a_whole_number_not_a_boolean() -> None:
    with pytest.raises(Unreadable):
        decode(ok(page(True)))


def test_a_text_exactly_at_the_limit_is_accepted_and_one_byte_more_is_not() -> None:
    def limited(text: str) -> list[PageContent]:
        return decode_text_answer(
            ok(page(text=text)), 1, 1, False, max_pages=50, max_page_text_bytes=10
        )

    assert limited("a" * 10)[0].text == "a" * 10
    with pytest.raises(Unreadable):
        limited("a" * 11)
    # Five characters are ten bytes: allowed. Six are twelve: refused, though under ten characters.
    assert limited("é" * 5)
    with pytest.raises(Unreadable):
        limited("é" * 6)


def test_a_tab_and_a_line_feed_are_allowed_in_text_but_other_control_characters_are_not() -> None:
    assert decode(ok(page(text="Total\t100\n200")))[0].text == "Total\t100\n200"
    for bad in ["a\r", "a\x00", "a\x1b", "a\x7f", "a\x85"]:
        with pytest.raises(Unreadable):
            decode(ok(page(text=bad)))


def test_a_page_with_no_text_must_have_no_fonts_and_no_text_separately() -> None:
    with pytest.raises(Unreadable):
        decode(ok(page(source="none", text="oops", fonts=[])))


def test_fonts_must_be_a_list_of_at_most_sixty_four() -> None:
    font = {"kind": "type1", "has_to_unicode": False, "encoding": "none", "embedded": False}
    assert len(decode(ok(page(fonts=[font] * 64)))[0].fonts) == 64
    with pytest.raises(Unreadable):
        decode(ok(page(fonts=[font] * 65)))
    for bad in (5, None, {"kind": "type1"}):
        with pytest.raises(Unreadable):
            decode(ok(page(fonts=bad)))


def test_both_flags_of_a_font_must_be_true_or_false_exactly() -> None:
    for key in ("has_to_unicode", "embedded"):
        font = {"kind": "type1", "has_to_unicode": False, "encoding": "none", "embedded": False}
        font[key] = 1
        with pytest.raises(Unreadable):
            decode(ok(page(fonts=[font])))


def test_a_word_box_must_not_start_above_or_left_of_the_page() -> None:
    for x, y in [(-1, 0), (0, -1)]:
        word = {"start": 0, "end": 5, "x": x, "y": y, "w": 1, "h": 1}
        with pytest.raises(Unreadable):
            decode(ok(page(words=[word])), words=True)


def test_a_word_that_runs_past_the_end_of_the_text_is_unreadable_even_without_white_space() -> None:
    word = {"start": 6, "end": 99, "x": 0, "y": 0, "w": 1, "h": 1}
    with pytest.raises(Unreadable):
        decode(ok(page(words=[word])), words=True)
