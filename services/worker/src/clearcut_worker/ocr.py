# Written by Kone Tshivhinda & Claude
"""Reading a page by OCR with Tesseract, and checking the text layer against it (ADR-0016).

Tesseract is a program, run with a fixed list of arguments and no shell (a file name is
only ever a file name), in the child's private scratch directory, with an empty
environment apart from where its language data is, and under the deadline of the
request. Its answer is a TSV file with a line for every word it found; that file is
derived from a hostile picture, so it is parsed strictly and every word is cleaned.

Two languages are read together, English and Afrikaans: South African invoices often
are, or are bilingual. An installation without both is not used at all (the worker
refuses to start with it), rather than reading Afrikaans badly.

The worker reports facts. Besides the words it reports an **agreement**: of the distinct
words of two letters or more in the PDF's own text layer, how many (per mille) OCR also
read. A text layer that lies (a scrambled font, digits swapped but still digits) shares
few of its words with what a reader sees. What a low agreement means is decided by
``clearcut-core``, not here.
"""

from __future__ import annotations

import logging
import os
import re
import shutil
import subprocess
import time
import unicodedata
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path

from .errors import TooSlow, Unreadable
from .limits import scrub_environment
from .pagetext import (
    MAX_PAGE_TEXT_BYTES,
    MAX_WORDS_PER_PAGE,
    PageContent,
    PageTooLarge,
    Word,
    extract_pages,
    is_noncharacter,
    permille_span,
)
from .render import render_page

log = logging.getLogger("clearcut_worker")

LANGUAGES = "eng+afr"
REQUIRED_LANGUAGES = ("eng", "afr")
# Tesseract reads small print best at about 300 dpi (the pictures the model sees are 150).
OCR_DPI = 300
OCR_MAX_EDGE = 4000
OCR_MAX_IMAGE_BYTES = 20 * 1024 * 1024
MAX_TSV_BYTES = 32 * 1024 * 1024
_PROBE_TIMEOUT_SECONDS = 15.0

_TSV_HEADER = (
    "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext"
)
_WORD_LEVEL = 5
_INTEGER = re.compile(r"^-?[0-9]{1,9}$")
_NUMBER = re.compile(r"^-?[0-9]{1,6}(\.[0-9]{1,9})?$")
_TOKEN = re.compile(r"[^\W_]+")
_LINE_BREAK = re.compile(r"\r?\n")


@dataclass(frozen=True)
class OcrEngine:
    """How to start Tesseract: the program (and anything before its arguments), and its data."""

    command: tuple[str, ...]
    tessdata_dir: str | None = None


@dataclass(frozen=True)
class OcrWord:
    """A word Tesseract found, in the pixels of the picture it was given."""

    text: str
    left: int
    top: int
    width: int
    height: int
    # Thousandths: Tesseract's 0 to 100 times ten.
    confidence: int
    # Which block, paragraph and line it is on, so lines can be put back together.
    line: tuple[int, int, int]


# ------------------------------------------------------------------ finding the program ----


def _environment(tessdata_dir: str | None) -> dict[str, str]:
    """What the program sees: nothing but the short allowlist, one thread, and its data."""
    env = dict(os.environ)
    scrub_environment(env)
    env["OMP_THREAD_LIMIT"] = "1"
    if tessdata_dir is not None:
        env["TESSDATA_PREFIX"] = tessdata_dir
    return env


def _data_arguments(tessdata_dir: str | None) -> list[str]:
    return [] if tessdata_dir is None else ["--tessdata-dir", tessdata_dir]


def probe_engine(
    command: Sequence[str],
    tessdata_dir: str | None,
    *,
    timeout: float = _PROBE_TIMEOUT_SECONDS,
) -> OcrEngine | None:
    """The engine, if it starts and has English and Afrikaans; None otherwise."""
    argv = [*command, "--list-langs", *_data_arguments(tessdata_dir)]
    try:
        done = subprocess.run(  # noqa: S603 - a fixed argument list, no shell
            argv,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            timeout=timeout,
            env=_environment(tessdata_dir),
            check=False,
        )
    except (OSError, subprocess.SubprocessError):
        log.warning("the OCR program could not be started to list its languages")
        return None
    if done.returncode != 0:
        log.warning("the OCR program failed when asked for its languages")
        return None
    listed = {line.strip() for line in done.stdout.decode("utf-8", "replace").splitlines()}
    if not all(language in listed for language in REQUIRED_LANGUAGES):
        log.warning("the OCR program lacks English or Afrikaans language data")
        return None
    return OcrEngine(tuple(command), tessdata_dir)


def find_engine(configured: str | None, tessdata_dir: str | None) -> OcrEngine | None:
    """The configured program, or ``tesseract`` from the path, if it can be used."""
    program = configured or shutil.which("tesseract")
    if not program:
        return None
    return probe_engine((program,), tessdata_dir)


# ------------------------------------------------------------------ running it ----


def run_tesseract(
    engine: OcrEngine,
    image: bytes,
    media_type: str,
    *,
    workdir: Path,
    deadline: float,
    languages: str = LANGUAGES,
    max_tsv_bytes: int = MAX_TSV_BYTES,
) -> bytes:
    """Gives Tesseract the picture and returns the TSV it writes."""
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TooSlow
    name = "page.jpg" if media_type == "image/jpeg" else "page.png"
    (workdir / name).write_bytes(image)
    argv = [
        *engine.command,
        name,
        "result",
        "-l",
        languages,
        "--oem",
        "1",
        "--psm",
        "3",
        *_data_arguments(engine.tessdata_dir),
        "tsv",
    ]
    try:
        done = subprocess.run(  # noqa: S603 - a fixed argument list, no shell
            argv,
            cwd=workdir,
            env=_environment(engine.tessdata_dir),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=remaining,
            check=False,
        )
    except subprocess.TimeoutExpired:
        raise TooSlow from None
    except OSError:
        raise Unreadable from None
    if done.returncode != 0:
        log.debug("the OCR program failed with status %d", done.returncode)
        raise Unreadable
    result = workdir / "result.tsv"
    try:
        size = result.stat().st_size
    except OSError:
        raise Unreadable from None
    if size > max_tsv_bytes:
        raise Unreadable
    return result.read_bytes()


# ------------------------------------------------------------------ reading the answer ----


def _integer(field: str) -> int:
    if not _INTEGER.match(field):
        raise Unreadable
    return int(field)


def _confidence(field: str) -> float:
    if not _NUMBER.match(field):
        raise Unreadable
    return float(field)


def _clean(text: str) -> str:
    """The word without control characters, noncharacters or white space inside it."""
    return "".join(
        c
        for c in text
        if not (c.isspace() or unicodedata.category(c) == "Cc" or is_noncharacter(c))
    )


def parse_tsv(data: bytes, *, max_words: int = MAX_WORDS_PER_PAGE) -> list[OcrWord]:
    """The words in what Tesseract wrote, strictly: anything else than its format is unreadable."""
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        raise Unreadable from None
    lines = _LINE_BREAK.split(text)
    if not lines or lines[0] != _TSV_HEADER:
        raise Unreadable
    words: list[OcrWord] = []
    for line in lines[1:]:
        if line == "":
            continue
        fields = line.split("\t", 11)
        if len(fields) != 12:
            raise Unreadable
        level, _page, block, paragraph, line_number, _word = (_integer(f) for f in fields[:6])
        left, top, width, height = (_integer(f) for f in fields[6:10])
        confidence = _confidence(fields[10])
        if level != _WORD_LEVEL or confidence < 0 or width <= 0 or height <= 0:
            continue
        cleaned = _clean(fields[11])
        if not cleaned:
            continue
        if len(words) >= max_words:
            raise PageTooLarge
        words.append(
            OcrWord(
                cleaned,
                left,
                top,
                width,
                height,
                min(1000, round(confidence * 10)),
                (block, paragraph, line_number),
            )
        )
    return words


def assemble_ocr(
    words: Sequence[OcrWord],
    size: tuple[int, int],
    *,
    max_words: int = MAX_WORDS_PER_PAGE,
    max_text_bytes: int = MAX_PAGE_TEXT_BYTES,
) -> tuple[str, tuple[Word, ...]]:
    """The page's text (words on a line separated by a space, lines by a line feed) and the
    words in it, with UTF-8 byte offsets and boxes in thousandths of the picture."""
    if len(words) > max_words:
        raise PageTooLarge
    width, height = size
    pieces: list[str] = []
    located: list[Word] = []
    position = 0
    previous: tuple[int, int, int] | None = None
    for word in words:
        if previous is not None:
            separator = " " if word.line == previous else "\n"
            pieces.append(separator)
            position += 1
        previous = word.line
        start = position
        position += len(word.text.encode("utf-8"))
        if position > max_text_bytes:
            raise PageTooLarge
        pieces.append(word.text)
        x, w = permille_span(word.left, word.left + word.width, width)
        y, h = permille_span(word.top, word.top + word.height, height)
        located.append(Word(start, position, x, y, w, h, word.confidence))
    return "".join(pieces), tuple(located)


# ------------------------------------------------------------------ comparing the two ----


def _tokens(text: str) -> set[str]:
    return {token for token in _TOKEN.findall(text.casefold()) if len(token) >= 2}


def agreement_per_mille(text_layer: str, ocr_text: str) -> int | None:
    """Of the distinct words (two letters or digits or more) in the text layer, how many per
    mille OCR also read; None when the text layer has no such words to compare."""
    layer = _tokens(text_layer)
    if not layer:
        return None
    return len(layer & _tokens(ocr_text)) * 1000 // len(layer)


# ------------------------------------------------------------------ a page ----


def ocr_page(
    data: bytes,
    page: int,
    *,
    engine: OcrEngine,
    agreement: bool,
    max_pages: int,
    deadline: float,
    workdir: Path,
    max_page_text_bytes: int = MAX_PAGE_TEXT_BYTES,
    max_words_per_page: int = MAX_WORDS_PER_PAGE,
) -> PageContent:
    """Page ``page`` (1-based) read by OCR, with the agreement with its text layer if asked.

    Raises the same named refusals as reading text and drawing a page.
    """
    if page < 1:
        raise ValueError("the page must be 1 or later")
    [layer] = extract_pages(data, page, page, words=False, max_pages=max_pages, deadline=deadline)
    picture = render_page(
        data,
        page,
        dpi=OCR_DPI,
        fmt="png",
        max_edge=OCR_MAX_EDGE,
        max_bytes=OCR_MAX_IMAGE_BYTES,
        max_pages=max_pages,
        deadline=deadline,
    )
    answer = run_tesseract(
        engine, picture.data, picture.media_type, workdir=workdir, deadline=deadline
    )
    found = parse_tsv(answer, max_words=max_words_per_page)
    text, located = assemble_ocr(
        found,
        (picture.width, picture.height),
        max_words=max_words_per_page,
        max_text_bytes=max_page_text_bytes,
    )
    return PageContent(
        page=page,
        source="ocr",
        text=text,
        width_pt=layer.width_pt,
        height_pt=layer.height_pt,
        fonts=(),
        words=located,
        agreement=(
            agreement_per_mille(layer.text, text)
            if agreement and layer.source == "text_layer"
            else None
        ),
    )
