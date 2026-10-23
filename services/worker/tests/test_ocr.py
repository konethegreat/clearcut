# Written by Kone Tshivhinda & Claude
"""Reading a page by OCR with Tesseract, and checking the text layer against it (ADR-0016)."""

from __future__ import annotations

import time
from pathlib import Path

import pytest

from clearcut_worker.errors import Encrypted, NotAPdf, TooSlow, Unreadable
from clearcut_worker.images import ImageTooLarge
from clearcut_worker.ocr import (
    LANGUAGES,
    OCR_DPI,
    OcrEngine,
    OcrWord,
    agreement_per_mille,
    assemble_ocr,
    find_engine,
    ocr_page,
    parse_tsv,
    probe_engine,
    run_tesseract,
)
from clearcut_worker.pagetext import PageContent, PageOutOfRange, PageTooLarge, Word

from .conftest import make_encrypted_pdf
from .ocrs import HEADER, fake_engine, real_engine, records, row, tsv
from .pdfs import one_page, pages_of_text, text_at
from .pictures import claims_to_be, png, text_picture

FAR = time.monotonic() + 3600
PNG_SIGNATURE_HEX = "89504e470d0a1a0a"

needs_tesseract = pytest.mark.skipif(
    real_engine() is None, reason="Tesseract with English and Afrikaans is not installed"
)


def words_of(*rows: str) -> list[OcrWord]:
    return parse_tsv(tsv(*rows).encode())


# ---------- reading what Tesseract wrote ----------


def test_the_words_are_the_level_five_rows_with_confidence_in_thousandths() -> None:
    found = words_of(
        row("Invoice", 100, 50, 200, 40, 96.5, word=1),
        row("INV-1", 320, 50, 150, 40, 91.0, word=2),
    )
    assert found == [
        OcrWord("Invoice", 100, 50, 200, 40, 965, (1, 1, 1)),
        OcrWord("INV-1", 320, 50, 150, 40, 910, (1, 1, 1)),
    ]


def test_the_lines_and_blocks_each_word_belongs_to_are_kept() -> None:
    found = words_of(
        row("a", 1, 1, 5, 5, block=1, par=2, line=3),
        row("b", 1, 1, 5, 5, block=4, par=5, line=6),
    )
    assert [w.line for w in found] == [(1, 2, 3), (4, 5, 6)]


def test_rows_that_are_not_words_or_have_no_word_in_them_are_left_out() -> None:
    found = words_of(
        row("", 10, 10, 50, 20, -1),  # a word row with nothing in it
        row("noise", 10, 10, 50, 20, -1),  # no confidence at all
        row("line level", 10, 10, 50, 20, 90, level=4),
        row("ok", 10, 10, 50, 20, 90),
        row("flat", 10, 10, 0, 20, 90),  # no width
        row("thin", 10, 10, 50, 0, 90),  # no height
    )
    assert [w.text for w in found] == ["ok"]


def test_spaces_of_every_kind_inside_a_word_are_removed() -> None:
    found = words_of(row("a b", 1, 1, 5, 5), row("a" + chr(0xA0) + "b", 1, 1, 5, 5))
    assert [w.text for w in found] == ["ab", "ab"]


def test_a_confidence_of_zero_is_kept_and_one_over_a_hundred_is_capped() -> None:
    found = words_of(row("low", 1, 1, 5, 5, 0), row("high", 1, 1, 5, 5, 250))
    assert [w.confidence for w in found] == [0, 1000]


def test_windows_line_endings_and_a_missing_final_newline_are_fine() -> None:
    body = tsv(row("one", 1, 1, 5, 5), newline="\r\n").rstrip("\r\n").encode()
    assert [w.text for w in parse_tsv(body)] == ["one"]
    assert [w.text for w in parse_tsv(tsv(row("one", 1, 1, 5, 5)).encode() + b"\n\n")] == ["one"]


def test_a_header_only_file_has_no_words() -> None:
    assert parse_tsv(tsv().encode()) == []


def test_control_characters_and_white_space_inside_a_word_are_removed() -> None:
    found = words_of(
        row("a\x07b", 1, 1, 5, 5),
        row("c\td", 1, 1, 5, 5),
        row("\x01\x02", 1, 1, 5, 5),  # nothing left of it
        row("e￾f", 1, 1, 5, 5),  # a noncharacter
    )
    assert [w.text for w in found] == ["ab", "cd", "ef"]


def bad_row(line: str) -> bytes:
    return "\n".join([HEADER, line, ""]).encode()


@pytest.mark.parametrize(
    "data",
    [
        b"",
        b"not a tsv at all",
        b"level\tpage_num\n1\t1\n",  # the wrong header
        tsv().replace("conf", "confidence").encode(),
        bad_row("5\t1\t1\t1\t1\t1\t1\t1\t5\t5\t90"),  # a short row
        bad_row("a\t1\t1\t1\t1\t1\t1\t1\t5\t5\t90\tx"),  # the level is no number
        bad_row("5\t1\t1\t1\t1\t1\tleft\t1\t5\t5\t90\tx"),
        bad_row("5\t1\t1\t1\t1\t1\t1\t1\t5\t5\tnan\tx"),
        bad_row("5\t1\t1\t1\t1\t1\t1\t1\t5\t5\tinf\tx"),
        bad_row("5\t1\t1\t1\t1\t1\t1\t1\t5\t5\tabc\tx"),
        b"\xff\xfe\x00 not utf-8",
    ],
)
def test_a_file_that_is_not_what_tesseract_writes_is_unreadable(data: bytes) -> None:
    with pytest.raises(Unreadable):
        parse_tsv(data)


# ---------- putting the words together ----------


def test_lines_become_lines_of_text_and_words_know_their_place_in_it() -> None:
    found = words_of(
        row("Total", 100, 500, 100, 40, line=1, word=1),
        row("100.00", 220, 500, 120, 40, line=1, word=2),
        row("VAT", 100, 560, 80, 40, line=2, word=1),
    )
    text, words = assemble_ocr(found, (1000, 800))
    assert text == "Total 100.00\nVAT"
    assert [(w.start, w.end) for w in words] == [(0, 5), (6, 12), (13, 16)]
    raw = text.encode()
    assert [raw[w.start : w.end].decode() for w in words] == ["Total", "100.00", "VAT"]


def test_a_new_block_or_paragraph_starts_a_new_line() -> None:
    found = words_of(
        row("a", 1, 1, 5, 5, block=1, par=1, line=1),
        row("b", 1, 1, 5, 5, block=1, par=2, line=1),
        row("c", 1, 1, 5, 5, block=2, par=1, line=1),
        row("d", 6, 1, 5, 5, block=2, par=1, line=1, word=2),
    )
    assert assemble_ocr(found, (100, 100))[0] == "a\nb\nc d"


def test_offsets_count_utf8_bytes_not_characters() -> None:
    found = words_of(row("Café", 1, 1, 50, 5, word=1), row("au", 60, 1, 20, 5, word=2))
    text, words = assemble_ocr(found, (200, 100))
    assert text == "Café au"
    assert [(w.start, w.end) for w in words] == [(0, 5), (6, 8)]


def test_boxes_are_in_thousandths_of_the_picture_from_the_top_left() -> None:
    found = words_of(row("x", 200, 100, 400, 50))
    _text, [word] = assemble_ocr(found, (2000, 1000))
    assert (word.x, word.y, word.w, word.h) == (100, 100, 200, 50)


def test_boxes_are_rounded_outwards_and_kept_inside_the_picture() -> None:
    _t, [word] = assemble_ocr(words_of(row("x", 3, 3, 5, 5)), (7, 7))
    assert (word.x, word.w) == (428, 572)  # 3/7 down, and out to the edge
    _t, [outside] = assemble_ocr(words_of(row("x", -50, 10, 100, 10)), (1000, 1000))
    assert (outside.x, outside.w) == (0, 50)
    _t, [beyond] = assemble_ocr(words_of(row("x", 2000, 10, 100, 10)), (1000, 1000))
    assert (beyond.x, beyond.w) == (999, 1)


def test_the_confidence_travels_with_the_word() -> None:
    _t, [word] = assemble_ocr(words_of(row("x", 1, 1, 5, 5, 96.43)), (100, 100))
    assert word.confidence == 964


def test_no_words_is_no_text() -> None:
    assert assemble_ocr([], (100, 100)) == ("", ())


def test_a_page_with_too_many_words_or_too_much_text_is_refused_not_cut() -> None:
    found = words_of(*[row(f"w{n}", 1, 1, 5, 5, word=n + 1) for n in range(3)])
    assert len(assemble_ocr(found, (100, 100), max_words=3)[1]) == 3
    with pytest.raises(PageTooLarge):
        assemble_ocr(found, (100, 100), max_words=2)
    text = assemble_ocr(found, (100, 100))[0]
    assert assemble_ocr(found, (100, 100), max_text_bytes=len(text.encode()))[0] == text
    with pytest.raises(PageTooLarge):
        assemble_ocr(found, (100, 100), max_text_bytes=len(text.encode()) - 1)


# ---------- how well two readings of a page agree ----------


def test_the_same_words_agree_completely() -> None:
    assert (
        agreement_per_mille("Invoice INV-2024 Total 1500.00", "Invoice INV-2024 Total 1500.00")
        == 1000
    )


def test_agreement_is_the_share_of_the_text_layers_own_words_that_ocr_also_read() -> None:
    assert agreement_per_mille("total 100 paid", "total 100 paid extra words") == 1000
    assert agreement_per_mille("total 100 paid", "total 999 paid") == 666
    assert agreement_per_mille("total 100 paid", "nothing alike here") == 0


def test_digits_that_are_permuted_but_still_look_like_digits_lower_the_agreement() -> None:
    assert agreement_per_mille("Total 1500 Due 2400", "Total 5100 Due 4200") == 500


def test_case_punctuation_and_accents_in_the_layout_do_not_matter() -> None:
    assert agreement_per_mille("FAKTUUR: Totaal (BTW)", "faktuur totaal btw") == 1000
    assert agreement_per_mille("Café-au-lait", "café au lait") == 1000


def test_a_word_counts_once_however_often_it_is_repeated() -> None:
    assert agreement_per_mille("vat vat vat vat total", "total") == 500


def test_single_characters_are_too_noisy_to_count_and_underscores_split_words() -> None:
    assert agreement_per_mille("a 1 R total", "total") == 1000
    assert agreement_per_mille("sub_total", "sub total") == 1000


def test_a_text_layer_with_nothing_to_compare_has_no_agreement() -> None:
    for layer in ("", "   ", "a 1 .", "---"):
        assert agreement_per_mille(layer, "total 100") is None


def test_nothing_read_by_ocr_is_zero_agreement() -> None:
    assert agreement_per_mille("total 100", "") == 0


def test_agreement_rounds_down() -> None:
    assert agreement_per_mille("aa bb cc", "aa bb") == 666


# ---------- starting the program ----------


def image_bytes() -> bytes:
    return png(30, 20)


def test_the_program_is_started_with_a_fixed_argument_list_in_the_scratch_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("CLEARCUT_WORKER_TOKEN", "placeholder-secret-for-the-test-only")
    monkeypatch.setenv("SOME_OTHER_SECRET", "placeholder")
    engine = fake_engine(tmp_path / "engine", tsv=tsv(row("x", 1, 1, 5, 5)))
    work = tmp_path / "work"
    work.mkdir()
    result = run_tesseract(engine, image_bytes(), "image/png", workdir=work, deadline=FAR)
    assert b"x" in result
    [record] = records(tmp_path / "engine")
    assert record["argv"] == [
        "page.png",
        "result",
        "-l",
        LANGUAGES,
        "--oem",
        "1",
        "--psm",
        "3",
        "tsv",
    ]
    assert LANGUAGES == "eng+afr"
    assert Path(record["cwd"]).resolve() == work.resolve()
    assert record["image_head"] == PNG_SIGNATURE_HEX
    assert record["env"]["OMP_THREAD_LIMIT"] == "1"
    assert "CLEARCUT_WORKER_TOKEN" not in record["env"]
    assert "SOME_OTHER_SECRET" not in record["env"]


def test_a_jpeg_is_handed_over_under_a_jpeg_name(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine", tsv=tsv())
    work = tmp_path / "work"
    work.mkdir()
    run_tesseract(engine, b"\xff\xd8\xff\xe0 jpeg", "image/jpeg", workdir=work, deadline=FAR)
    [record] = records(tmp_path / "engine")
    assert record["argv"][0] == "page.jpg"


def test_a_data_directory_is_passed_on_the_command_line_and_in_the_environment(
    tmp_path: Path,
) -> None:
    engine = fake_engine(tmp_path / "engine", tsv=tsv())
    engine = OcrEngine(engine.command, str(tmp_path / "tessdata"))
    work = tmp_path / "work"
    work.mkdir()
    run_tesseract(engine, image_bytes(), "image/png", workdir=work, deadline=FAR)
    [record] = records(tmp_path / "engine")
    argv = record["argv"]
    assert argv[argv.index("--tessdata-dir") + 1] == str(tmp_path / "tessdata")
    assert argv[-1] == "tsv"
    assert record["env"]["TESSDATA_PREFIX"] == str(tmp_path / "tessdata")


def test_no_shell_is_involved_so_odd_file_names_are_just_names(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "a b & c ; d", tsv=tsv(row("ok", 1, 1, 5, 5)))
    work = tmp_path / "w k $(x)"
    work.mkdir()
    out = run_tesseract(engine, image_bytes(), "image/png", workdir=work, deadline=FAR)
    assert b"ok" in out


@pytest.mark.parametrize("scenario", [{"exit": 1}, {"no_output": True}, {"exit": 2, "tsv": "x"}])
def test_a_program_that_fails_or_writes_nothing_means_an_unreadable_page(
    tmp_path: Path, scenario: dict[str, object]
) -> None:
    engine = fake_engine(tmp_path / "engine", stderr="SECRET words from the program", **scenario)
    work = tmp_path / "work"
    work.mkdir()
    with pytest.raises(Unreadable) as caught:
        run_tesseract(engine, image_bytes(), "image/png", workdir=work, deadline=FAR)
    assert "SECRET" not in str(caught.value)


def test_a_program_that_is_not_there_is_an_unreadable_page_not_a_crash(tmp_path: Path) -> None:
    work = tmp_path / "work"
    work.mkdir()
    with pytest.raises(Unreadable):
        run_tesseract(
            OcrEngine((str(tmp_path / "no-such-program"),)),
            image_bytes(),
            "image/png",
            workdir=work,
            deadline=FAR,
        )


def test_a_hanging_program_is_killed_at_the_deadline(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine", hang=True)
    work = tmp_path / "work"
    work.mkdir()
    started = time.monotonic()
    with pytest.raises(TooSlow):
        run_tesseract(
            engine, image_bytes(), "image/png", workdir=work, deadline=time.monotonic() + 1.0
        )
    assert time.monotonic() - started < 15


def test_the_program_is_not_even_started_when_the_time_is_already_up(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine", tsv=tsv())
    work = tmp_path / "work"
    work.mkdir()
    with pytest.raises(TooSlow):
        run_tesseract(
            engine, image_bytes(), "image/png", workdir=work, deadline=time.monotonic() - 1
        )
    assert records(tmp_path / "engine") == []
    assert not (work / "page.png").exists()


def test_a_result_exactly_at_the_size_limit_is_read_and_one_byte_more_is_not(
    tmp_path: Path,
) -> None:
    body = tsv(row("x", 1, 1, 5, 5))
    size = len(body.encode())
    engine = fake_engine(tmp_path / "engine", tsv=body)
    work = tmp_path / "work"
    work.mkdir()
    assert run_tesseract(
        engine, image_bytes(), "image/png", workdir=work, deadline=FAR, max_tsv_bytes=size
    )
    with pytest.raises(Unreadable):
        run_tesseract(
            engine, image_bytes(), "image/png", workdir=work, deadline=FAR, max_tsv_bytes=size - 1
        )


def test_an_enormous_result_is_refused_without_being_read(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine", tsv=tsv(), big=5000)
    work = tmp_path / "work"
    work.mkdir()
    with pytest.raises(Unreadable):
        run_tesseract(
            engine, image_bytes(), "image/png", workdir=work, deadline=FAR, max_tsv_bytes=1000
        )


# ---------- finding out whether the program can be used ----------


def test_an_engine_with_english_and_afrikaans_can_be_used(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine", langs=["afr", "eng", "osd"])
    found = probe_engine(engine.command, None)
    assert found == OcrEngine(engine.command, None)
    [record] = records(tmp_path / "engine")
    assert record["argv"] == ["--list-langs"]


def test_listing_languages_also_runs_with_a_scrubbed_environment_and_the_data_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("CLEARCUT_WORKER_TOKEN", "placeholder-secret-for-the-test-only")
    engine = fake_engine(tmp_path / "engine")
    assert probe_engine(engine.command, "/data") is not None
    [record] = records(tmp_path / "engine")
    assert "CLEARCUT_WORKER_TOKEN" not in record["env"]
    assert record["env"]["TESSDATA_PREFIX"] == "/data"
    assert record["env"]["OMP_THREAD_LIMIT"] == "1"


def test_the_data_directory_is_used_when_listing_languages_too(tmp_path: Path) -> None:
    engine = fake_engine(tmp_path / "engine")
    assert probe_engine(engine.command, "/data") == OcrEngine(engine.command, "/data")
    [record] = records(tmp_path / "engine")
    assert record["argv"] == ["--list-langs", "--tessdata-dir", "/data"]


@pytest.mark.parametrize("langs", [["eng"], ["afr"], ["osd"], []])
def test_an_engine_missing_a_language_the_worker_needs_is_not_used(
    tmp_path: Path, langs: list[str]
) -> None:
    engine = fake_engine(tmp_path / "engine", langs=langs)
    assert probe_engine(engine.command, None) is None


def test_an_engine_that_fails_hangs_or_is_missing_is_not_used(tmp_path: Path) -> None:
    failing = fake_engine(tmp_path / "a", exit=1)
    assert probe_engine(failing.command, None) is None
    hanging = fake_engine(tmp_path / "b", hang_listing=True)
    started = time.monotonic()
    assert probe_engine(hanging.command, None, timeout=1.0) is None
    assert time.monotonic() - started < 15
    assert probe_engine((str(tmp_path / "missing"),), None) is None


def test_an_unconfigured_engine_is_looked_for_on_the_path_and_a_configured_one_is_used(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    probed: list[tuple[tuple[str, ...], str | None]] = []

    def fake_probe(command: tuple[str, ...], tessdata: str | None, **_: object) -> OcrEngine:
        probed.append((command, tessdata))
        return OcrEngine(command, tessdata)

    monkeypatch.setattr("clearcut_worker.ocr.probe_engine", fake_probe)
    monkeypatch.setattr("clearcut_worker.ocr.shutil.which", lambda name: None)
    assert find_engine(None, None) is None
    assert probed == []
    monkeypatch.setattr("clearcut_worker.ocr.shutil.which", lambda name: "/usr/bin/" + name)
    assert find_engine(None, "/data") == OcrEngine(("/usr/bin/tesseract",), "/data")
    assert find_engine("/opt/tess/bin/tesseract", None) == OcrEngine(
        ("/opt/tess/bin/tesseract",), None
    )


# ---------- a page, end to end, with the stand-in program ----------


def page_engine(tmp_path: Path, *rows: str, **scenario: object) -> OcrEngine:
    return fake_engine(tmp_path / "engine", tsv=tsv(*rows), **scenario)


def ocr(
    data: bytes, page: int, engine: OcrEngine, tmp_path: Path, **options: object
) -> PageContent:
    work = tmp_path / "work"
    work.mkdir(exist_ok=True)
    settings: dict[str, object] = {
        "engine": engine,
        "agreement": False,
        "max_pages": 50,
        "deadline": FAR,
        "workdir": work,
    }
    settings.update(options)
    return ocr_page(data, page, **settings)  # type: ignore[arg-type]


def test_a_page_comes_back_as_ocr_text_with_boxes_and_confidence(tmp_path: Path) -> None:
    engine = page_engine(
        tmp_path,
        row("Invoice", 83, 380, 150, 40, 96.5, word=1),
        row("INV-1", 250, 380, 100, 40, 88.0, word=2),
    )
    page = ocr(one_page(text_at(20, 100, "Invoice INV-1")), 1, engine, tmp_path)
    assert page.page == 1
    assert page.source == "ocr"
    assert page.text == "Invoice INV-1"
    assert (page.width_pt, page.height_pt) == (200.0, 200.0)
    assert page.fonts == ()
    assert page.agreement is None
    assert page.words is not None
    assert [(w.start, w.end, w.confidence) for w in page.words] == [(0, 7, 965), (8, 13, 880)]
    assert all(isinstance(w, Word) and w.w >= 1 and w.h >= 1 for w in page.words)


def test_the_page_is_handed_to_the_program_as_a_png_at_300_dpi(tmp_path: Path) -> None:
    engine = page_engine(tmp_path)
    ocr(one_page(""), 1, engine, tmp_path)
    [record] = records(tmp_path / "engine")
    assert OCR_DPI == 300
    assert record["image_head"] == PNG_SIGNATURE_HEX
    assert record["image_wh"] == [834, 834]  # 200 points at 300 dpi, rounded up


def test_a_later_page_is_the_one_read(tmp_path: Path) -> None:
    engine = page_engine(tmp_path, row("two", 10, 10, 50, 20))
    page = ocr(pages_of_text("one", "two"), 2, engine, tmp_path)
    assert page.page == 2


def test_a_page_with_no_words_found_is_an_empty_ocr_page(tmp_path: Path) -> None:
    page = ocr(one_page(""), 1, page_engine(tmp_path), tmp_path)
    assert (page.source, page.text, page.words) == ("ocr", "", ())


def test_agreement_is_only_worked_out_when_asked_for(tmp_path: Path) -> None:
    engine = page_engine(
        tmp_path, row("Invoice", 83, 380, 150, 40, word=1), row("INV-1", 250, 380, 100, 40, word=2)
    )
    pdf = one_page(text_at(20, 100, "Invoice INV-1"))
    assert ocr(pdf, 1, engine, tmp_path).agreement is None
    assert ocr(pdf, 1, engine, tmp_path, agreement=True).agreement == 1000


def test_agreement_falls_when_ocr_reads_something_else_than_the_text_layer(
    tmp_path: Path,
) -> None:
    engine = page_engine(
        tmp_path, row("Invoice", 83, 380, 150, 40), row("999", 250, 380, 100, 40, word=2)
    )
    pdf = one_page(text_at(20, 100, "Invoice 100"))
    assert ocr(pdf, 1, engine, tmp_path, agreement=True).agreement == 500


def test_a_page_with_no_text_layer_has_no_agreement_to_report(tmp_path: Path) -> None:
    engine = page_engine(tmp_path, row("Scanned", 83, 380, 150, 40))
    assert ocr(one_page(""), 1, engine, tmp_path, agreement=True).agreement is None


def test_a_picture_document_is_read_in_its_own_size(tmp_path: Path) -> None:
    engine = page_engine(tmp_path, row("Scanned", 5, 5, 20, 10))
    page = ocr(png(60, 30), 1, engine, tmp_path, agreement=True)
    assert (page.width_pt, page.height_pt) == (60.0, 30.0)
    assert page.agreement is None
    assert page.text == "Scanned"
    [record] = records(tmp_path / "engine")
    assert record["image_wh"] == [60, 30]


def test_what_cannot_be_read_is_refused_in_a_named_way(tmp_path: Path) -> None:
    engine = page_engine(tmp_path)
    with pytest.raises(NotAPdf):
        ocr(b"<html>nope</html>", 1, engine, tmp_path)
    with pytest.raises(Encrypted):
        ocr(make_encrypted_pdf(), 1, engine, tmp_path)
    with pytest.raises(Unreadable):
        ocr(b"%PDF-1.4\nSECRET garbage", 1, engine, tmp_path)
    with pytest.raises(ImageTooLarge):
        ocr(claims_to_be(30_000, 30_000), 1, engine, tmp_path)
    with pytest.raises(PageOutOfRange) as caught:
        ocr(pages_of_text("a", "b"), 3, engine, tmp_path)
    assert caught.value.pages == 2
    assert records(tmp_path / "engine") == []


def test_page_zero_is_a_programming_error(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="must be 1 or later"):
        ocr(pages_of_text("a"), 0, page_engine(tmp_path), tmp_path)


def test_a_deadline_that_has_passed_stops_it_before_the_program_is_started(tmp_path: Path) -> None:
    engine = page_engine(tmp_path)
    with pytest.raises(TooSlow):
        ocr(pages_of_text("a"), 1, engine, tmp_path, deadline=time.monotonic() - 1)
    assert records(tmp_path / "engine") == []


def test_a_page_with_more_text_than_allowed_is_refused_not_cut(tmp_path: Path) -> None:
    engine = page_engine(tmp_path, row("abcdefghij", 1, 1, 5, 5))
    with pytest.raises(PageTooLarge):
        ocr(pages_of_text("a"), 1, engine, tmp_path, max_page_text_bytes=5)


# ---------- the real program (run where it is installed) ----------


@needs_tesseract
def test_the_real_program_lists_english_and_afrikaans() -> None:
    engine = real_engine()
    assert engine is not None
    assert engine == probe_engine(engine.command, None)


@needs_tesseract
def test_the_real_program_reads_a_drawn_page_and_says_where_each_word_is(tmp_path: Path) -> None:
    engine = real_engine()
    assert engine is not None
    pdf = one_page(
        text_at(60, 700, "Invoice INV-2024", size=28) + text_at(60, 640, "Total 1500.00", size=28),
        media_box="[0 0 595 842]",
    )
    page = ocr(pdf, 1, engine, tmp_path)
    assert page.source == "ocr"
    assert (page.width_pt, page.height_pt) == (595.0, 842.0)
    lowered = page.text.lower()
    assert "invoice" in lowered
    assert "total" in lowered
    assert "1500" in page.text
    assert page.words is not None
    assert len(page.words) >= 4
    raw = page.text.encode()
    by_text = {raw[w.start : w.end].decode(): w for w in page.words}
    invoice = next(w for text, w in by_text.items() if text.lower().startswith("invoice"))
    total = next(w for text, w in by_text.items() if text.lower().startswith("total"))
    # "Invoice" is drawn about 142 points from the top, "Total" 60 points lower, near the left.
    assert 40 <= invoice.x <= 140
    assert 120 <= invoice.y <= 200
    assert total.y > invoice.y
    assert all(0 <= w.confidence <= 1000 for w in page.words if w.confidence is not None)
    assert max(w.confidence or 0 for w in page.words) > 700


@needs_tesseract
def test_the_real_program_agrees_with_an_honest_text_layer(tmp_path: Path) -> None:
    engine = real_engine()
    assert engine is not None
    pdf = one_page(
        text_at(60, 700, "Invoice INV-2024", size=28) + text_at(60, 640, "Total 1500.00", size=28),
        media_box="[0 0 595 842]",
    )
    page = ocr(pdf, 1, engine, tmp_path, agreement=True)
    assert page.agreement is not None
    assert page.agreement >= 800


@needs_tesseract
def test_the_real_program_disagrees_with_a_text_layer_that_says_something_else(
    tmp_path: Path,
) -> None:
    engine = real_engine()
    assert engine is not None
    # What can be seen says 100.00; the invisible layer a reader would copy says 999.00.
    seen = text_at(60, 700, "Total 100.00 paid", size=28)
    unseen = "BT /F1 28 Tf 3 Tr 60 640 Td (Total 999.00 paid) Tj ET\n"
    page = ocr(
        one_page(seen + unseen, media_box="[0 0 595 842]"), 1, engine, tmp_path, agreement=True
    )
    assert page.agreement is not None
    assert 300 <= page.agreement <= 900


@needs_tesseract
def test_the_real_program_reads_a_scanned_picture_including_afrikaans_words(
    tmp_path: Path,
) -> None:
    engine = real_engine()
    assert engine is not None
    page = ocr(text_picture("Faktuur 2024-017", "Totaal BTW 15%"), 1, engine, tmp_path)
    lowered = page.text.lower()
    assert "faktuur" in lowered
    assert "totaal" in lowered
    assert page.agreement is None


def test_the_confidence_is_rounded_to_the_nearest_thousandth_not_cut() -> None:
    found = words_of(row("x", 1, 1, 5, 5, 96.46), row("y", 1, 1, 5, 5, 96.44, word=2))
    assert [w.confidence for w in found] == [965, 964]


def test_a_word_cap_on_the_file_stops_the_reading_early() -> None:
    rows = [row(f"w{n}", 1, 1, 5, 5, word=n + 1) for n in range(3)]
    assert len(parse_tsv(tsv(*rows).encode(), max_words=3)) == 3
    with pytest.raises(PageTooLarge):
        parse_tsv(tsv(*rows).encode(), max_words=2)


def test_digits_that_are_not_ascii_are_not_numbers_in_the_file() -> None:
    line = "\t".join(["5", "1", "1", "1", "1", "1", "١", "1", "5", "5", "90", "x"])  # noqa: RUF001
    with pytest.raises(Unreadable):
        parse_tsv("\n".join([HEADER, line, ""]).encode())


def test_a_column_that_is_far_too_long_a_number_is_unreadable() -> None:
    line = "\t".join(["5", "1", "1", "1", "1", "1", "1" * 20, "1", "5", "5", "90", "x"])
    with pytest.raises(Unreadable):
        parse_tsv("\n".join([HEADER, line, ""]).encode())


def test_a_row_with_a_blank_line_between_is_still_read() -> None:
    body = tsv(row("a", 1, 1, 5, 5)).replace("\n5\t", "\n\n5\t")
    assert [w.text for w in parse_tsv(body.encode())] == ["a"]


def test_agreement_is_one_thousand_only_when_every_word_is_read_back() -> None:
    assert agreement_per_mille("aa bb", "aa bb cc dd") == 1000
    assert agreement_per_mille("aa bb cc dd", "aa bb") == 500


def test_it_is_the_asked_for_page_that_is_drawn_measured_and_compared(tmp_path: Path) -> None:
    from .pdfs import PageSpec, PdfBuilder

    pdf = PdfBuilder(
        pages=[
            PageSpec(content=text_at(20, 100, "Invoice INV-1"), media_box="[0 0 200 200]"),
            PageSpec(content=text_at(20, 50, "Total 500.00"), media_box="[0 0 100 100]"),
        ]
    ).build()
    engine = page_engine(
        tmp_path, row("Total", 10, 10, 60, 20), row("500.00", 80, 10, 90, 20, word=2)
    )
    page = ocr(pdf, 2, engine, tmp_path, agreement=True)
    assert (page.width_pt, page.height_pt) == (100.0, 100.0)
    assert page.agreement == 1000  # the second page's own text layer, not the first
    [record] = records(tmp_path / "engine")
    assert record["image_wh"] == [417, 417]  # 100 points at 300 dpi, rounded up


def test_a_large_page_is_read_at_full_resolution_up_to_four_thousand_pixels(
    tmp_path: Path,
) -> None:
    engine = page_engine(tmp_path)
    ocr(one_page("", media_box="[0 0 595 842]"), 1, engine, tmp_path)
    [record] = records(tmp_path / "engine")
    width, height = record["image_wh"]
    assert 2480 <= width <= 2481
    assert 3508 <= height <= 3509
