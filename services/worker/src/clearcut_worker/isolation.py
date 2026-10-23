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
import math
import multiprocessing
import os
import sys
import tempfile
import threading
import time
from collections.abc import Callable, Iterator
from pathlib import Path
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
from .limits import ChildLimits, apply_limits, scrub_environment

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
        # Waiting for the first byte is not a deadline for the whole answer: a child
        # that has been taken over can start a message and never finish it, which would
        # leave the parent blocked in ``recv_bytes`` for ever. So a watchdog kills the
        # child at the deadline; the pipe then ends and the read fails.
        expired = threading.Event()

        def stop_the_child() -> None:
            expired.set()
            process.kill()

        watchdog = threading.Timer(timeout_seconds + grace_seconds, stop_the_child)
        watchdog.daemon = True
        watchdog.start()
        try:
            if not receiver.poll(timeout_seconds + grace_seconds):
                raise TooSlow
            try:
                return receiver.recv_bytes(max_answer_bytes)
            except (EOFError, OSError):
                raise (TooSlow if expired.is_set() else Unreadable) from None
        finally:
            watchdog.cancel()
            receiver.close()
            if process.is_alive():
                process.kill()
            process.join(5)


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
    return _decode(raw, max_pages)


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
