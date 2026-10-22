# Written by Kone Tshivhinda & Claude
"""Stand-ins for the inspection child process, used to test how the parent copes.

They live in an importable module because the child is started with "spawn",
which has to import the function it runs.
"""

from __future__ import annotations

import os
import time
from multiprocessing.connection import Connection


def hang(conn: Connection, data: bytes, max_pages: int, timeout_seconds: float) -> None:
    """A parser stuck forever on a hostile file."""
    time.sleep(120)


def crash(conn: Connection, data: bytes, max_pages: int, timeout_seconds: float) -> None:
    """A parser that dies hard (segfault, out of memory): no answer, no clean exit."""
    os._exit(3)


def nonsense(conn: Connection, data: bytes, max_pages: int, timeout_seconds: float) -> None:
    """A child that answers with something that is not an answer."""
    conn.send_bytes(b"SECRET this is not json")
    conn.close()


def unknown_error(conn: Connection, data: bytes, max_pages: int, timeout_seconds: float) -> None:
    """A child that reports an error kind the parent has never heard of."""
    conn.send_bytes(b'["error", "SomethingNew"]')
    conn.close()


def wrong_shape(conn: Connection, data: bytes, max_pages: int, timeout_seconds: float) -> None:
    """Valid JSON that is not a valid answer."""
    conn.send_bytes(b'["ok", -1, 5]')
    conn.close()


def too_much(conn: Connection, data: bytes, max_pages: int, timeout_seconds: float) -> None:
    """A child that tries to push a huge message at the parent."""
    conn.send_bytes(b'["error", "' + b"x" * 1_000_000 + b'"]')
    conn.close()


class _Evil:
    """Unpickling this would create the marker file: it runs code."""

    def __reduce__(self) -> tuple[object, tuple[object, ...]]:
        return (open, (os.environ["CLEARCUT_TEST_MARKER"], "w"))


def malicious_pickle(conn: Connection, data: bytes, max_pages: int, timeout_seconds: float) -> None:
    """A compromised child trying to run code in the parent through pickle."""
    conn.send(_Evil())  # Connection.send pickles the object
    conn.close()


def echo(conn: Connection, data: bytes, max_pages: int, timeout_seconds: float) -> None:
    """A child whose answer is exactly the bytes it was given as the 'document'."""
    conn.send_bytes(data)
    conn.close()
