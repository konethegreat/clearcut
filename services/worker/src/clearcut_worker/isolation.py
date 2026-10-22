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

import json
import multiprocessing
import time
from collections.abc import Callable
from typing import Protocol, TypeGuard

from .inspect import (
    Encrypted,
    InspectError,
    Inspection,
    NotAPdf,
    TooManyPages,
    TooSlow,
    Unreadable,
    inspect_pdf,
)

# The child's whole answer is smaller than this. Anything longer is refused.
_MAX_ANSWER_BYTES = 256

_REFUSALS: dict[str, type[InspectError]] = {
    "NotAPdf": NotAPdf,
    "Encrypted": Encrypted,
    "Unreadable": Unreadable,
    "TooSlow": TooSlow,
}


class AnswerChannel(Protocol):
    """The child's end of the pipe, described by what the child does with it.

    (The concrete class differs between Windows and Linux, so the code relies on
    this shape instead of on either class.)
    """

    def send_bytes(self, buf: bytes) -> None: ...

    def send(self, obj: object) -> None: ...

    def close(self) -> None: ...


ChildTarget = Callable[[AnswerChannel, bytes, int, float], None]


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


def inspect_isolated(
    data: bytes,
    max_pages: int,
    timeout_seconds: float,
    *,
    grace_seconds: float = 2.0,
    target: ChildTarget = _child_main,
) -> Inspection:
    """Inspects ``data`` in a child process, killing it if it overruns.

    ``timeout_seconds`` is the child's own deadline (it gives up cleanly between
    pages). The parent waits ``grace_seconds`` longer, for the child to start
    and answer, then kills it. ``target`` exists so tests can stand in a
    misbehaving child.
    """
    context = multiprocessing.get_context("spawn")
    receiver, sender = context.Pipe(duplex=False)
    process = context.Process(
        target=target, args=(sender, data, max_pages, timeout_seconds), daemon=True
    )
    process.start()
    # Close the parent's copy of the child's end, so that a child that dies is
    # seen as end-of-file instead of silence.
    sender.close()
    try:
        if not receiver.poll(timeout_seconds + grace_seconds):
            raise TooSlow
        try:
            raw = receiver.recv_bytes(_MAX_ANSWER_BYTES)
        except (EOFError, OSError):
            raise Unreadable from None
        return _decode(raw, max_pages)
    finally:
        receiver.close()
        if process.is_alive():
            process.kill()
        process.join(5)


def _decode(raw: bytes, max_pages: int) -> Inspection:
    """The child's answer, checked field by field. Anything unexpected is "unreadable"."""
    try:
        answer = json.loads(raw)
    except ValueError:
        raise Unreadable from None
    if isinstance(answer, list) and len(answer) == 3 and answer[0] == "ok":
        pages, with_text = answer[1], answer[2]
        if _count(pages) and _count(with_text) and with_text <= pages:
            return Inspection(pages=pages, pages_with_text=with_text)
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
