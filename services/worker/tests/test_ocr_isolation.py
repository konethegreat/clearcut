# Written by Kone Tshivhinda & Claude
"""OCR in a child process, and not believing what the child says (ADR-0016)."""

from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Any

import pytest

from clearcut_worker.images import ImageTooLarge
from clearcut_worker.inspect import Encrypted, NotAPdf, TooManyPages, TooSlow, Unreadable
from clearcut_worker.isolation import decode_ocr_answer, ocr_isolated
from clearcut_worker.ocr import OcrEngine, ocr_page
from clearcut_worker.pagetext import PageContent, PageOutOfRange, PageTooLarge, Word
from clearcut_worker.render import PageTooBig

from . import children
from .conftest import make_encrypted_pdf
from .ocrs import fake_engine, records, row, tsv
from .pdfs import one_page, pages_of_text, text_at
from .pictures import claims_to_be, png
from .test_isolation import no_children_left

FAR = time.monotonic() + 3600
TWO_WORDS = (
    row("Invoice", 83, 380, 150, 40, 96.5, word=1),
    row("INV-1", 250, 380, 100, 40, 88.0, word=2),
)


def read(
    data: bytes, engine: OcrEngine, page: int = 1, *, agreement: bool = False, **options: Any
) -> PageContent:
    settings: dict[str, Any] = {"max_pages": 50, "timeout_seconds": 30.0}
    settings.update(options)
    return ocr_isolated(data, page, engine=engine, agreement=agreement, **settings)


# ---------- the whole trip ----------


def test_the_answer_from_the_child_equals_reading_in_this_process(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine", tsv=tsv(*TWO_WORDS))
    pdf = one_page(text_at(20, 100, "Invoice INV-1"))
    work = tmp_path / "work"
    work.mkdir()
    inside = ocr_page(
        pdf, 1, engine=engine, agreement=True, max_pages=50, deadline=FAR, workdir=work
    )
    assert read(pdf, engine, agreement=True) == inside
    assert inside.agreement == 1000
    assert no_children_left()


def test_the_program_runs_in_the_childs_scratch_directory_not_in_the_workers(
    tmp_path: Path,
) -> None:
    engine = fake_engine(tmp_path / "engine", tsv=tsv())
    read(pages_of_text("a"), engine)
    [record] = records(tmp_path / "engine")
    assert Path(record["cwd"]).resolve() != Path.cwd().resolve()
    assert not Path(record["cwd"]).exists()  # deleted afterwards


def test_a_picture_document_makes_the_trip_too(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine", tsv=tsv(row("Scanned", 5, 5, 20, 10)))
    page = read(png(60, 30), engine, agreement=True)
    assert page.text == "Scanned"
    assert page.agreement is None


def test_every_kind_of_refusal_survives_the_trip_back_from_the_child(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine", tsv=tsv())
    with pytest.raises(NotAPdf):
        read(b"<html>nope</html>", engine)
    with pytest.raises(Encrypted):
        read(make_encrypted_pdf(), engine)
    with pytest.raises(Unreadable):
        read(b"%PDF-1.4\nSECRET garbage", engine)
    with pytest.raises(TooManyPages) as many:
        read(pages_of_text("a", "b", "c"), engine, max_pages=2)
    assert many.value.max_pages == 2
    with pytest.raises(PageOutOfRange) as out:
        read(pages_of_text("a", "b"), engine, 3)
    assert out.value.pages == 2
    with pytest.raises(PageTooBig):
        read(one_page("", media_box="[0 0 14400 14400]"), engine)
    with pytest.raises(ImageTooLarge):
        read(claims_to_be(30_000, 30_000), engine)
    with pytest.raises(TooSlow):
        read(pages_of_text("a"), engine, timeout_seconds=0.0)


def test_a_page_with_too_much_text_is_refused_through_the_child(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine", tsv=tsv(row("abcdefghij", 1, 1, 5, 5)))
    with pytest.raises(PageTooLarge):
        read(pages_of_text("a"), engine, max_page_text_bytes=5)


def test_a_program_that_fails_is_an_unreadable_page(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine", exit=1)
    with pytest.raises(Unreadable):
        read(pages_of_text("a"), engine)


def test_a_program_that_hangs_is_stopped_at_the_deadline_and_leaves_nothing_behind(
    tmp_path: Path,
) -> None:
    engine = fake_engine(tmp_path / "engine", hang=True)
    started = time.monotonic()
    with pytest.raises(TooSlow):
        read(pages_of_text("a"), engine, timeout_seconds=1.5)
    assert time.monotonic() - started < 20
    assert no_children_left()


# ---------- hostile children ----------


def test_an_ocr_child_stuck_for_ever_is_killed_at_the_hard_limit(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine")
    started = time.monotonic()
    with pytest.raises(TooSlow):
        read(
            pages_of_text("a"),
            engine,
            timeout_seconds=0.5,
            grace_seconds=0.5,
            target=children.ocr_hang,
        )
    assert time.monotonic() - started < 15
    assert no_children_left()


def test_an_ocr_child_that_dies_hard_is_an_unreadable_document(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine")
    with pytest.raises(Unreadable):
        read(pages_of_text("a"), engine, target=children.ocr_crash)
    assert no_children_left()


def test_an_answer_over_the_size_cap_is_unreadable(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine")
    with pytest.raises(Unreadable):
        read(b"x" * 2048, engine, target=children.ocr_echo, max_answer_bytes=1024)


# ---------- checking an answer field by field ----------


def entry(**overrides: Any) -> dict[str, Any]:
    base: dict[str, Any] = {
        "page": 1,
        "source": "ocr",
        "text": "Total 100",
        "width_pt": 595.0,
        "height_pt": 842.5,
        "fonts": [],
        "words": [
            {"start": 0, "end": 5, "x": 10, "y": 20, "w": 100, "h": 30, "confidence": 900},
            {"start": 6, "end": 9, "x": 120, "y": 20, "w": 100, "h": 30, "confidence": 850},
        ],
    }
    base.update(overrides)
    return base


def ok(*pages: dict[str, Any]) -> bytes:
    return json.dumps(["ok", list(pages)], ensure_ascii=False).encode("utf-8")


def decode(raw: bytes, page: int = 1, agreement: bool = False, **options: Any) -> PageContent:
    settings: dict[str, Any] = {"max_pages": 50}
    settings.update(options)
    return decode_ocr_answer(raw, page, agreement, **settings)


def test_a_well_formed_answer_is_accepted_and_becomes_page_content() -> None:
    assert decode(ok(entry())) == PageContent(
        page=1,
        source="ocr",
        text="Total 100",
        width_pt=595.0,
        height_pt=842.5,
        fonts=(),
        words=(Word(0, 5, 10, 20, 100, 30, 900), Word(6, 9, 120, 20, 100, 30, 850)),
        agreement=None,
    )


def test_a_page_with_no_words_is_an_empty_page() -> None:
    page = decode(ok(entry(text="", words=[])))
    assert (page.text, page.words) == ("", ())


def test_the_agreement_must_be_there_when_asked_for_and_only_then() -> None:
    assert decode(ok(entry(agreement=730)), agreement=True).agreement == 730
    assert decode(ok(entry(agreement=None)), agreement=True).agreement is None
    assert decode(ok(entry(agreement=0)), agreement=True).agreement == 0
    assert decode(ok(entry(agreement=1000)), agreement=True).agreement == 1000
    with pytest.raises(Unreadable):
        decode(ok(entry()), agreement=True)
    with pytest.raises(Unreadable):
        decode(ok(entry(agreement=500)), agreement=False)


@pytest.mark.parametrize("agreement", [-1, 1001, True, 1.5, "500", [], {}])
def test_an_agreement_that_is_not_a_share_per_mille_is_unreadable(agreement: Any) -> None:
    with pytest.raises(Unreadable):
        decode(ok(entry(agreement=agreement)), agreement=True)


@pytest.mark.parametrize(
    "answer",
    [
        b"",
        b"not json",
        b"null",
        b"[]",
        b'["ok"]',
        b'["ok", {}]',
        b'["ok", []]',  # no page at all
        b'["ok", [1]]',
        b'["ok", [], "extra"]',
        b'["maybe", []]',
        b'["error"]',
        b'["error", "SomethingNew"]',
        b"\xff\xfe broken utf-8",
        b'["ok", [{"page": NaN}]]',
    ],
)
def test_nonsense_is_unreadable(answer: bytes) -> None:
    with pytest.raises(Unreadable):
        decode(answer)


def test_exactly_one_page_is_expected_and_it_must_be_the_one_asked_for() -> None:
    with pytest.raises(Unreadable):
        decode(ok(entry(), entry(page=2)))
    with pytest.raises(Unreadable):
        decode(ok(entry(page=2)))
    assert decode(ok(entry(page=3)), page=3).page == 3
    with pytest.raises(Unreadable):
        decode(ok(entry(page=True)))


@pytest.mark.parametrize(
    "overrides",
    [
        {"source": "text_layer"},
        {"source": "none"},
        {"source": "OCR"},
        {"source": 5},
        {"text": 5},
        {"text": "Total\r\n100"},
        {"text": "bell\x07 100"},
        {"text": "", "words": entry()["words"]},  # words with no text
        {"text": "Total 100", "words": []},  # text with no words
        {"width_pt": 0},
        {"width_pt": -3.5},
        {"width_pt": 1e9},
        {"height_pt": "tall"},
        {"width_pt": True},
        {"fonts": [{"kind": "type1"}]},
        {"fonts": "none"},
        {"extra": "key"},
    ],
)
def test_a_page_entry_that_is_not_what_was_promised_is_unreadable(
    overrides: dict[str, Any],
) -> None:
    with pytest.raises(Unreadable):
        decode(ok(entry(**overrides)))


def test_the_missing_keys_are_unreadable_too() -> None:
    for key in ("page", "source", "text", "width_pt", "height_pt", "fonts", "words"):
        without = entry()
        del without[key]
        with pytest.raises(Unreadable):
            decode(ok(without))


def test_the_text_limit_counts_bytes_and_is_exact() -> None:
    words = [{"start": 0, "end": 10, "x": 1, "y": 1, "w": 5, "h": 5, "confidence": 900}]
    page = entry(text="a" * 10, words=words)
    assert decode(ok(page), max_page_text_bytes=10).text == "a" * 10
    with pytest.raises(Unreadable):
        decode(ok(page), max_page_text_bytes=9)


def word(**overrides: Any) -> dict[str, Any]:
    base = {"start": 0, "end": 5, "x": 0, "y": 0, "w": 1, "h": 1, "confidence": 500}
    base.update(overrides)
    return base


@pytest.mark.parametrize(
    "words",
    [
        [word(confidence=-1)],
        [word(confidence=1001)],
        [word(confidence=True)],
        [word(confidence=90.5)],
        [word(confidence="90")],
        [word(confidence=None)],
        [{k: v for k, v in word().items() if k != "confidence"}],  # no confidence at all
        [word(start=5, end=3)],
        [word(start=0, end=0)],
        [word(end=99)],
        [word(start=0, end=9)],  # spans the space
        [word(), word(start=3, end=9)],  # overlapping
        [word(x=-1)],
        [word(y=-1)],
        [word(x=999, w=2)],
        [word(y=999, h=2)],
        [word(w=0)],
        [word(h=0)],
        [word(t="x")],
        ["word"],
        "words",
        5,
        None,
    ],
)
def test_a_word_that_does_not_fit_the_text_or_the_page_or_lacks_confidence_is_unreadable(
    words: Any,
) -> None:
    with pytest.raises(Unreadable):
        decode(ok(entry(words=words)))


def test_a_flood_of_words_is_refused() -> None:
    first = word(start=0, end=5)
    second = word(start=6, end=9)
    assert len(decode(ok(entry(words=[first, second]))).words or ()) == 2
    with pytest.raises(Unreadable):
        decode(ok(entry(words=[first, second])), max_words_per_page=1)


def test_the_error_names_come_back_as_themselves() -> None:
    for name, expected in [
        ("NotAPdf", NotAPdf),
        ("Encrypted", Encrypted),
        ("Unreadable", Unreadable),
        ("TooSlow", TooSlow),
        ("PageTooLarge", PageTooLarge),
        ("PageTooBig", PageTooBig),
        ("ImageTooLarge", ImageTooLarge),
        ("TooManyPages", TooManyPages),
    ]:
        with pytest.raises(expected):
            decode(json.dumps(["error", name]).encode())
    with pytest.raises(PageOutOfRange) as caught:
        decode(json.dumps(["error", "PageOutOfRange", 7]).encode())
    assert caught.value.pages == 7


def test_confidence_may_be_exactly_nothing_or_exactly_certain() -> None:
    words = [
        {"start": 0, "end": 5, "x": 10, "y": 20, "w": 100, "h": 30, "confidence": 0},
        {"start": 6, "end": 9, "x": 120, "y": 20, "w": 100, "h": 30, "confidence": 1000},
    ]
    page = decode(ok(entry(words=words)))
    assert [w.confidence for w in page.words or ()] == [0, 1000]


def test_the_data_directory_reaches_the_program_through_the_child(tmp_path: Path) -> None:
    fake = fake_engine(tmp_path / "engine", tsv=tsv())
    engine = OcrEngine(fake.command, str(tmp_path / "tessdata"))
    read(pages_of_text("a"), engine)
    [record] = records(tmp_path / "engine")
    argv = record["argv"]
    assert argv[argv.index("--tessdata-dir") + 1] == str(tmp_path / "tessdata")
    assert record["env"]["TESSDATA_PREFIX"] == str(tmp_path / "tessdata")
