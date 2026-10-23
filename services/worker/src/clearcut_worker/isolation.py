# Written by Kone Tshivhinda & Claude
"""Inspecting a document in a child process that the parent can kill.

A document is hostile input, and a parser can hang, run out of memory or crash
on it. Checking a deadline between pages (``inspect.py``) cannot stop a parser
that is stuck inside one call, and a crash would take the whole worker with it.
So each inspection runs in a **separate process** that the parent watches:

- **A hard time limit.** If the child has not answered within the timeout plus a
  short grace period, the parent kills it. Nothing a file does can keep the
  worker busy longer.
- **Crashes stay in the child.** A segfault or out-of-memory kill in the parser
  ends the child; the parent reports "could not be read" and carries on serving.
- **The child cannot hurt the parent through its answer.** The answer is a few
  bytes of JSON with a hard size limit, checked field by field. It is never
  unpickled (unpickling is running code), so even a fully compromised parser
  cannot use the channel to escape.

The child is started with ``spawn`` (a fresh interpreter), not ``fork``, which
is unsafe in a process with threads. A fresh interpreter also inherits none of
the parent's state. Memory limits for the child belong to the container
(P1-07); this module guarantees time and containment.
"""

from __future__ import annotations

import contextlib
import dataclasses
import json
import logging
import math
import multiprocessing
import os
import signal
import struct
import sys
import tempfile
import threading
import time
import unicodedata
from collections.abc import Callable, Iterator
from pathlib import Path
from typing import Any, Protocol, TypeGuard

from .images import MAX_IMAGE_BYTES, EncodedImage, ImageTooLarge, sniff
from .inspect import (
    MEDIA_TYPES,
    Encrypted,
    InspectError,
    Inspection,
    NotAPdf,
    TooManyPages,
    TooSlow,
    Unreadable,
    inspect_pdf,
)
from .limits import ChildLimits, apply_limits, scrub_environment
from .pagetext import (
    MAX_PAGE_TEXT_BYTES,
    MAX_PAGES_PER_CALL,
    MAX_WORDS_PER_PAGE,
    FontFact,
    PageContent,
    PageOutOfRange,
    PageTooLarge,
    Word,
    extract_pages,
)
from .render import PageTooBig, render_page

log = logging.getLogger("clearcut_worker")

# The child's whole answer is smaller than this. Anything longer is refused.
_MAX_ANSWER_BYTES = 256

_REFUSALS: dict[str, type[InspectError]] = {
    "NotAPdf": NotAPdf,
    "Encrypted": Encrypted,
    "Unreadable": Unreadable,
    "TooSlow": TooSlow,
    "ImageTooLarge": ImageTooLarge,
}


class AnswerChannel(Protocol):
    """The child's end of the pipe, described by what the child does with it.

    (The concrete class differs between Windows and Linux, so the code relies on
    this shape instead of on either class.)
    """

    def send_bytes(self, buf: bytes) -> None: ...

    def send(self, obj: object) -> None: ...

    def close(self) -> None: ...


# What runs in the child: it is given its end of the pipe, then the arguments.
ChildTarget = Callable[..., None]

# The limits a child runs under unless told otherwise.
DEFAULT_LIMITS = ChildLimits()


def _child_main(conn: AnswerChannel, data: bytes, max_pages: int, timeout_seconds: float) -> None:
    """What runs in the child: inspect, and send back a tiny JSON answer."""
    deadline = time.monotonic() + timeout_seconds
    try:
        try:
            result = inspect_pdf(data, max_pages, deadline)
            answer: list[object] = ["ok", result.pages, result.pages_with_text]
        except TooManyPages:
            answer = ["error", "TooManyPages"]
        except InspectError as error:
            answer = ["error", type(error).__name__]
        except BaseException:
            # Anything else the parser does is, to the caller, an unreadable document.
            answer = ["error", "Unreadable"]
        conn.send_bytes(json.dumps(answer).encode())
    finally:
        conn.close()


# Only one child is being started at a time while the environment is swapped.
_START_LOCK = threading.Lock()


@contextlib.contextmanager
def clean_environment_for_start() -> Iterator[None]:
    """Makes the worker's own environment the scrubbed one while a child is started.

    A child inherits the environment of the moment it is started, and a program
    keeps that starting block (on Linux it can be read from ``/proc/self/environ``)
    whatever it later removes from ``os.environ``. So scrubbing inside the child
    is not enough: the secrets must not be in the environment when the child is
    created. For the instant that takes, the worker's variables other than the
    allowlist are set aside, and they are put back afterwards, also if starting
    fails. The lock makes concurrent starts take turns.
    """
    with _START_LOCK:
        saved = dict(os.environ)
        removed = scrub_environment(os.environ)
        try:
            yield
        finally:
            for name in removed:
                os.environ[name] = saved[name]


def effective_limits(
    limits: ChildLimits, *, timeout_seconds: float, grace_seconds: float
) -> ChildLimits:
    """The limits to give a child: never more processor time than the parent will wait for."""
    ceiling = math.ceil(timeout_seconds + grace_seconds) + 1
    return dataclasses.replace(limits, cpu_seconds=min(limits.cpu_seconds, ceiling))


def _child_entry(
    target: ChildTarget,
    conn: AnswerChannel,
    args: tuple[object, ...],
    limits: ChildLimits | None,
    limiter: Callable[[ChildLimits], object],
    workdir: str,
) -> None:
    """What every child runs first: shed secrets, move into its own directory, set its limits.

    Anything that goes wrong here ends the child with no answer and **no
    message**: the parent reports an unreadable document, and nothing about the
    file is printed anywhere.
    """
    try:
        # Lead a session and a process group of its own, so that the parent can stop
        # the child and everything it starts in one signal (there is no such thing on Windows).
        if sys.platform != "win32":
            os.setsid()
        # Relative entries on the module path would stop resolving after the move.
        sys.path[:] = [os.path.abspath(entry) for entry in sys.path]
        scrub_environment(os.environ)
        os.chdir(workdir)
        for name in ("TMPDIR", "TEMP", "TMP"):
            os.environ[name] = workdir
        if limits is not None:
            limiter(limits)
        target(conn, *args)
    except BaseException:
        os._exit(1)


def _kill_tree(process: multiprocessing.process.BaseProcess) -> None:
    """Stops the child and, where there are process groups, everything it started.

    A child that has been taken over can start helper processes that keep the
    answer pipe open; killing only the child would leave the parent waiting on a
    pipe that never closes. The child leads its own process group, so one signal
    reaches them all. This runs before the child is reaped, so its process id
    cannot have been given to anyone else.
    """
    pid = process.pid
    if sys.platform != "win32" and pid is not None:
        with contextlib.suppress(ProcessLookupError, PermissionError):
            if os.getpgid(pid) == pid:
                os.killpg(pid, signal.SIGKILL)
    process.kill()


def run_in_child(
    target: ChildTarget,
    args: tuple[object, ...],
    *,
    timeout_seconds: float,
    max_answer_bytes: int,
    grace_seconds: float = 2.0,
    limits: ChildLimits | None = None,
    limiter: Callable[[ChildLimits], object] = apply_limits,
    workdir_parent: Path | None = None,
) -> bytes:
    """Runs ``target(conn, *args)`` in a child process; returns the bytes it sends back.

    The child starts with a scrubbed environment, in a private scratch directory
    that is deleted afterwards (even if the child is killed), under ``limits``
    when given. The parent waits ``timeout_seconds`` plus a short grace period,
    then kills it (``TooSlow``). A child that dies, answers more than
    ``max_answer_bytes``, or fails to set its limits gives ``Unreadable``.
    Nothing is unpickled: the answer is bytes, and the caller checks it.
    """
    child_limits = (
        None
        if limits is None
        else effective_limits(limits, timeout_seconds=timeout_seconds, grace_seconds=grace_seconds)
    )
    context = multiprocessing.get_context("spawn")
    with tempfile.TemporaryDirectory(
        prefix="clearcut-worker-", dir=workdir_parent, ignore_cleanup_errors=True
    ) as workdir:
        receiver, sender = context.Pipe(duplex=False)
        process = context.Process(
            target=_child_entry,
            args=(target, sender, args, child_limits, limiter, workdir),
            daemon=True,
        )
        with clean_environment_for_start():
            process.start()
        # Close the parent's copy of the child's end, so that a child that dies is
        # seen as end-of-file instead of silence.
        sender.close()
        # The deadline covers the whole answer, not just its first byte: a child that has
        # been taken over can start a message and never finish it, or leave a helper
        # process holding the pipe open after it has been killed. So the read happens in
        # a helper thread that the parent waits for with a time limit; whatever happens
        # to the pipe, the parent is never stuck in it.
        outcome: dict[str, bytes] = {}

        def read_answer() -> None:
            # The reader owns its end of the pipe and closes it itself, when it is done.
            # Closing it from another thread while this one is still inside a read could
            # leave the read working on a descriptor number that by then belongs to some
            # other request's pipe.
            try:
                with contextlib.suppress(EOFError, OSError):
                    outcome["bytes"] = receiver.recv_bytes(max_answer_bytes)
            finally:
                receiver.close()

        reader = threading.Thread(target=read_answer, name="clearcut-answer-reader", daemon=True)
        reader.start()
        try:
            reader.join(timeout_seconds + grace_seconds)
            if reader.is_alive():
                raise TooSlow
            if "bytes" not in outcome:
                raise Unreadable
            return outcome["bytes"]
        finally:
            _kill_tree(process)
            # A helper that inherited the child's end of the exit signal keeps ``join``
            # waiting until it ends too, so this wait is short; an unreaped child is
            # collected the next time a process is started.
            process.join(2)
            if reader.is_alive():
                reader.join(1)
            if reader.is_alive():
                # Only a helper that left the process group still holds the pipe; the
                # container's limits (no network, a process limit) are what contain it.
                log.warning("an answer reader is still blocked after the child was stopped")


def inspect_isolated(
    data: bytes,
    max_pages: int,
    timeout_seconds: float,
    *,
    grace_seconds: float = 2.0,
    target: ChildTarget = _child_main,
    limits: ChildLimits | None = DEFAULT_LIMITS,
) -> Inspection:
    """Inspects ``data`` in a child process, killing it if it overruns.

    ``timeout_seconds`` is the child's own deadline (it gives up cleanly between
    pages). The parent waits ``grace_seconds`` longer, for the child to start
    and answer, then kills it. ``target`` exists so tests can stand in a
    misbehaving child.
    """
    raw = run_in_child(
        target,
        (data, max_pages, timeout_seconds),
        timeout_seconds=timeout_seconds,
        max_answer_bytes=_MAX_ANSWER_BYTES,
        grace_seconds=grace_seconds,
        limits=limits,
    )
    # The kind of document is the parent's own reading of the bytes it sent, never
    # something the child says.
    return _decode(raw, max_pages, MEDIA_TYPES.get(sniff(data) or "", "application/pdf"))


def _decode(raw: bytes, max_pages: int, media_type: str = "application/pdf") -> Inspection:
    """The child's answer, checked field by field. Anything unexpected is "unreadable"."""
    try:
        answer = json.loads(raw)
    except ValueError:
        raise Unreadable from None
    if isinstance(answer, list) and len(answer) == 3 and answer[0] == "ok":
        pages, with_text = answer[1], answer[2]
        if _count(pages) and _count(with_text) and with_text <= pages:
            return Inspection(pages=pages, pages_with_text=with_text, media_type=media_type)
    if isinstance(answer, list) and len(answer) == 2 and answer[0] == "error":
        name = answer[1]
        if name == "TooManyPages":
            raise TooManyPages(max_pages)
        refusal = _REFUSALS.get(name) if isinstance(name, str) else None
        if refusal is not None:
            raise refusal
    raise Unreadable


def _count(value: object) -> TypeGuard[int]:
    """A real, non-negative whole number (``True`` is not one)."""
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


# ------------------------------------------------------------------ reading text ----

# The most the parent will read back from a text-reading child.
DEFAULT_TEXT_ANSWER_BYTES = 32 * 1024 * 1024

_FONT_KINDS = frozenset({"type1", "truetype", "type0", "type3", "other"})
_FONT_ENCODINGS = frozenset(
    {"none", "winansi", "macroman", "standard", "identity", "custom", "other"}
)
_MAX_PAGE_POINTS = 100_000.0
_MAX_FONTS = 64

_TEXT_REFUSALS: dict[str, type[InspectError]] = {
    "NotAPdf": NotAPdf,
    "Encrypted": Encrypted,
    "Unreadable": Unreadable,
    "TooSlow": TooSlow,
    "PageTooLarge": PageTooLarge,
    "ImageTooLarge": ImageTooLarge,
}


def _page_to_json(page: PageContent) -> dict[str, object]:
    entry: dict[str, object] = {
        "page": page.page,
        "source": page.source,
        "text": page.text,
        "width_pt": page.width_pt,
        "height_pt": page.height_pt,
        "fonts": [
            {
                "kind": f.kind,
                "has_to_unicode": f.has_to_unicode,
                "encoding": f.encoding,
                "embedded": f.embedded,
            }
            for f in page.fonts
        ],
    }
    if page.words is not None:
        entry["words"] = [
            {"start": w.start, "end": w.end, "x": w.x, "y": w.y, "w": w.w, "h": w.h}
            for w in page.words
        ]
    return entry


def _text_child(
    conn: AnswerChannel,
    data: bytes,
    first: int,
    last: int | None,
    words: bool,
    max_pages: int,
    timeout_seconds: float,
    max_page_text_bytes: int,
) -> None:
    """What runs in the child: read the pages and send back one JSON answer."""
    deadline = time.monotonic() + timeout_seconds
    try:
        answer: list[object]
        try:
            pages = extract_pages(
                data,
                first,
                last,
                words=words,
                max_pages=max_pages,
                deadline=deadline,
                max_page_text_bytes=max_page_text_bytes,
            )
            answer = ["ok", [_page_to_json(p) for p in pages]]
        except TooManyPages:
            answer = ["error", "TooManyPages"]
        except PageOutOfRange as error:
            answer = ["error", "PageOutOfRange", error.pages]
        except InspectError as error:
            answer = ["error", type(error).__name__]
        except BaseException:
            answer = ["error", "Unreadable"]
        conn.send_bytes(json.dumps(answer, ensure_ascii=False, allow_nan=False).encode("utf-8"))
    finally:
        conn.close()


def text_isolated(
    data: bytes,
    first: int,
    last: int | None,
    *,
    words: bool,
    max_pages: int,
    timeout_seconds: float,
    grace_seconds: float = 2.0,
    target: ChildTarget = _text_child,
    limits: ChildLimits | None = DEFAULT_LIMITS,
    max_answer_bytes: int = DEFAULT_TEXT_ANSWER_BYTES,
    max_page_text_bytes: int = MAX_PAGE_TEXT_BYTES,
) -> list[PageContent]:
    """Reads the text of pages ``first`` to ``last`` in a child process.

    The child's answer is checked field by field (see ``decode_text_answer``)
    before anything in it is believed.
    """
    raw = run_in_child(
        target,
        (data, first, last, words, max_pages, timeout_seconds, max_page_text_bytes),
        timeout_seconds=timeout_seconds,
        max_answer_bytes=max_answer_bytes,
        grace_seconds=grace_seconds,
        limits=limits,
    )
    return decode_text_answer(
        raw, first, last, words, max_pages=max_pages, max_page_text_bytes=max_page_text_bytes
    )


def _reject_constant(_: str) -> object:
    raise ValueError("not a JSON number")


def _whole(value: object) -> TypeGuard[int]:
    return isinstance(value, int) and not isinstance(value, bool)


def _points(value: object) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise Unreadable
    number = float(value)
    if not 0 < number <= _MAX_PAGE_POINTS:
        raise Unreadable
    return number


def decode_text_answer(
    raw: bytes,
    first: int,
    last: int | None,
    words: bool,
    *,
    max_pages: int,
    max_page_text_bytes: int = MAX_PAGE_TEXT_BYTES,
    max_words_per_page: int = MAX_WORDS_PER_PAGE,
) -> list[PageContent]:
    """The child's answer, checked field by field. Anything unexpected is "unreadable"."""
    try:
        answer = json.loads(raw, parse_constant=_reject_constant)
    except (ValueError, RecursionError):
        raise Unreadable from None
    if not isinstance(answer, list) or not answer:
        raise Unreadable
    if answer[0] == "error":
        raise _decode_error(answer, max_pages)
    if answer[0] != "ok" or len(answer) != 2 or not isinstance(answer[1], list):
        raise Unreadable
    entries: list[Any] = answer[1]
    if last is None:
        if not 1 <= len(entries) <= MAX_PAGES_PER_CALL:
            raise Unreadable
        expected = list(range(first, first + len(entries)))
    else:
        expected = list(range(first, last + 1))
    if len(entries) != len(expected):
        raise Unreadable
    return [
        _decode_page(entry, number, words, max_page_text_bytes, max_words_per_page)
        for entry, number in zip(entries, expected, strict=True)
    ]


def _decode_error(answer: list[Any], max_pages: int) -> InspectError:
    name = answer[1] if len(answer) > 1 else None
    if len(answer) == 2 and name == "TooManyPages":
        return TooManyPages(max_pages)
    if len(answer) == 2 and isinstance(name, str) and name in _TEXT_REFUSALS:
        return _TEXT_REFUSALS[name]()
    if len(answer) == 3 and name == "PageOutOfRange" and _whole(answer[2]) and answer[2] >= 0:
        return PageOutOfRange(answer[2])
    return Unreadable()


def _decode_page(
    entry: Any, number: int, words: bool, max_text: int, max_words: int
) -> PageContent:
    keys = {"page", "source", "text", "width_pt", "height_pt", "fonts"}
    if words:
        keys.add("words")
    if not isinstance(entry, dict) or set(entry) != keys:
        raise Unreadable
    if not _whole(entry["page"]) or entry["page"] != number:
        raise Unreadable
    text = entry["text"]
    if not isinstance(text, str) or len(text.encode("utf-8", "surrogatepass")) > max_text:
        raise Unreadable
    # Carriage returns are control characters too, so they are refused here as well.
    if any(unicodedata.category(c) == "Cc" and c not in "\n\t" for c in text):
        raise Unreadable
    source = entry["source"]
    if source == "text_layer":
        if not text.strip():
            raise Unreadable
    elif source == "none":
        if text or entry["fonts"] != []:
            raise Unreadable
    else:
        raise Unreadable
    width, height = _points(entry["width_pt"]), _points(entry["height_pt"])
    fonts = _decode_fonts(entry["fonts"])
    located = _decode_words(entry["words"], text, max_words) if words else None
    return PageContent(
        page=number,
        source=source,
        text=text,
        width_pt=width,
        height_pt=height,
        fonts=fonts,
        words=located,
    )


def _decode_fonts(value: Any) -> tuple[FontFact, ...]:
    if not isinstance(value, list) or len(value) > _MAX_FONTS:
        raise Unreadable
    fonts = []
    for item in value:
        if not isinstance(item, dict) or set(item) != {
            "kind",
            "has_to_unicode",
            "encoding",
            "embedded",
        }:
            raise Unreadable
        if (
            item["kind"] not in _FONT_KINDS
            or item["encoding"] not in _FONT_ENCODINGS
            or not isinstance(item["has_to_unicode"], bool)
            or not isinstance(item["embedded"], bool)
        ):
            raise Unreadable
        fonts.append(
            FontFact(item["kind"], item["has_to_unicode"], item["encoding"], item["embedded"])
        )
    return tuple(fonts)


def _decode_words(value: Any, text: str, max_words: int) -> tuple[Word, ...]:
    if not isinstance(value, list) or len(value) > max_words:
        raise Unreadable
    encoded = text.encode("utf-8")
    found: list[Word] = []
    previous_end = 0
    for item in value:
        if not isinstance(item, dict) or set(item) != {"start", "end", "x", "y", "w", "h"}:
            raise Unreadable
        numbers = [item[k] for k in ("start", "end", "x", "y", "w", "h")]
        if not all(_whole(n) for n in numbers):
            raise Unreadable
        start, end, x, y, w, h = numbers
        if not (previous_end <= start < end <= len(encoded)):
            raise Unreadable
        try:
            piece = encoded[start:end].decode("utf-8")
        except UnicodeDecodeError:
            raise Unreadable from None
        if any(c.isspace() for c in piece):
            raise Unreadable
        if not (x >= 0 and y >= 0 and w >= 1 and h >= 1 and x + w <= 1000 and y + h <= 1000):
            raise Unreadable
        found.append(Word(start, end, x, y, w, h))
        previous_end = end
    return tuple(found)


# ------------------------------------------------------------------ drawing pages ----

# The most the parent will read back from a rendering child: the biggest picture it
# accepts, and a few bytes of header.
DEFAULT_RENDER_ANSWER_BYTES = MAX_IMAGE_BYTES + 1024

_IMAGE_FRAME = b"CCIMG1"
_ERROR_FRAME = b"CCERR1"
_IMAGE_HEADER_BYTES = len(_IMAGE_FRAME) + 1 + 8
_PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
_PNG_END = b"\x00\x00\x00\x00IEND\xaeB`\x82"
_JPEG_START = b"\xff\xd8\xff"
_JPEG_END = b"\xff\xd9"

_RENDER_REFUSALS: dict[str, type[InspectError]] = {
    "NotAPdf": NotAPdf,
    "Encrypted": Encrypted,
    "Unreadable": Unreadable,
    "TooSlow": TooSlow,
    "PageTooBig": PageTooBig,
    "ImageTooLarge": ImageTooLarge,
}


def _render_child(
    conn: AnswerChannel,
    data: bytes,
    page: int,
    dpi: float,
    fmt: str,
    max_edge: int,
    max_pages: int,
    timeout_seconds: float,
    max_bytes: int,
) -> None:
    """What runs in the child: draw the page and send back one framed answer.

    The picture travels as raw bytes after a short header, not as JSON, so the parent
    never has to parse it.
    """
    deadline = time.monotonic() + timeout_seconds
    try:
        answer: bytes
        try:
            image = render_page(
                data,
                page,
                dpi=dpi,
                fmt=fmt,
                max_edge=max_edge,
                max_bytes=max_bytes,
                max_pages=max_pages,
                deadline=deadline,
            )
            kind = b"P" if image.media_type == "image/png" else b"J"
            answer = _IMAGE_FRAME + kind + struct.pack(">II", image.width, image.height)
            answer += image.data
        except TooManyPages:
            answer = _ERROR_FRAME + b"TooManyPages"
        except PageOutOfRange as error:
            answer = _ERROR_FRAME + b"PageOutOfRange:" + str(error.pages).encode("ascii")
        except InspectError as error:
            answer = _ERROR_FRAME + type(error).__name__.encode("ascii")
        except BaseException:
            answer = _ERROR_FRAME + b"Unreadable"
        conn.send_bytes(answer)
    finally:
        conn.close()


def render_isolated(
    data: bytes,
    page: int,
    *,
    dpi: float,
    fmt: str,
    max_edge: int,
    max_pages: int,
    timeout_seconds: float,
    grace_seconds: float = 2.0,
    target: ChildTarget = _render_child,
    limits: ChildLimits | None = DEFAULT_LIMITS,
    max_answer_bytes: int = DEFAULT_RENDER_ANSWER_BYTES,
    max_bytes: int = MAX_IMAGE_BYTES,
) -> EncodedImage:
    """Draws page ``page`` in a child process and returns the picture.

    The child's answer is checked (``decode_render_answer``) before anything in it is
    believed; the picture itself is never decoded by the parent.
    """
    raw = run_in_child(
        target,
        (data, page, dpi, fmt, max_edge, max_pages, timeout_seconds, max_bytes),
        timeout_seconds=timeout_seconds,
        max_answer_bytes=max_answer_bytes,
        grace_seconds=grace_seconds,
        limits=limits,
    )
    return decode_render_answer(
        raw, fmt=fmt, max_edge=max_edge, max_bytes=max_bytes, max_pages=max_pages
    )


def decode_render_answer(
    raw: bytes, *, fmt: str, max_edge: int, max_bytes: int, max_pages: int
) -> EncodedImage:
    """The child's answer, checked. Anything unexpected is "unreadable".

    The picture bytes are only measured: the right opening bytes, the right closing
    bytes, a size inside the limit, and (for a PNG) the width and height its own header
    states. Pillow never sees them in the parent.
    """
    if raw.startswith(_ERROR_FRAME):
        raise _decode_render_error(raw[len(_ERROR_FRAME) :], max_pages)
    if not raw.startswith(_IMAGE_FRAME) or len(raw) <= _IMAGE_HEADER_BYTES:
        raise Unreadable
    kind = raw[len(_IMAGE_FRAME) : len(_IMAGE_FRAME) + 1]
    width, height = struct.unpack(">II", raw[len(_IMAGE_FRAME) + 1 : _IMAGE_HEADER_BYTES])
    image = raw[_IMAGE_HEADER_BYTES:]
    if not (1 <= width <= max_edge and 1 <= height <= max_edge):
        raise Unreadable
    if len(image) > max_bytes:
        raise Unreadable
    if kind == b"P" and fmt == "png":
        if not (
            image.startswith(_PNG_SIGNATURE)
            and image.endswith(_PNG_END)
            and image[12:16] == b"IHDR"
            and struct.unpack(">II", image[16:24]) == (width, height)
        ):
            raise Unreadable
        return EncodedImage(image, "image/png", width, height)
    if kind == b"J":
        if not (
            image.startswith(_JPEG_START)
            and image.endswith(_JPEG_END)
            and _jpeg_size(image) == (width, height)
        ):
            raise Unreadable
        return EncodedImage(image, "image/jpeg", width, height)
    raise Unreadable


# The JPEG markers that start a frame header (the ones that state the picture's size).
_JPEG_FRAME_MARKERS = frozenset({0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD})
_JPEG_FRAME_MARKERS |= frozenset({0xCE, 0xCF})
_MAX_JPEG_SEGMENTS = 1000


def _jpeg_size(image: bytes) -> tuple[int, int] | None:
    """The width and height in a JPEG's frame header, found by walking its segments.

    Only the segment lengths are followed, never the picture data: the walk stops at the
    frame header, at the start of the scan, or after a fixed number of segments.
    """
    position = 2  # after the start-of-image marker
    for _ in range(_MAX_JPEG_SEGMENTS):
        if position + 4 > len(image) or image[position] != 0xFF:
            return None
        marker = image[position + 1]
        if marker == 0xFF:  # a fill byte before a marker
            position += 1
            continue
        if marker == 0x01 or 0xD0 <= marker <= 0xD8:  # markers with no length
            position += 2
            continue
        if marker in (0xD9, 0xDA):  # the end, or the scan before any frame header
            return None
        length = (image[position + 2] << 8) | image[position + 3]
        if length < 2:
            return None
        if marker in _JPEG_FRAME_MARKERS:
            if length < 8 or position + 9 > len(image):
                return None
            height = (image[position + 5] << 8) | image[position + 6]
            width = (image[position + 7] << 8) | image[position + 8]
            return width, height
        position += 2 + length
    return None


def _decode_render_error(rest: bytes, max_pages: int) -> InspectError:
    try:
        name = rest.decode("ascii")
    except UnicodeDecodeError:
        return Unreadable()
    if name == "TooManyPages":
        return TooManyPages(max_pages)
    refusal = _RENDER_REFUSALS.get(name)
    if refusal is not None:
        return refusal()
    prefix = "PageOutOfRange:"
    if name.startswith(prefix):
        digits = name[len(prefix) :]
        if digits.isascii() and digits.isdigit() and len(digits) <= 9:
            return PageOutOfRange(int(digits))
    return Unreadable()
