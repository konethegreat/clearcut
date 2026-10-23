# Written by Kone Tshivhinda & Claude
"""Stand-ins for a worker child process, for testing the harness around it.

They live in an importable module because the child is started with "spawn",
which has to import the function it runs.
"""

from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path

from clearcut_worker.isolation import AnswerChannel
from clearcut_worker.limits import ChildLimits


def report_environment(conn: AnswerChannel, *names: str) -> None:
    """Answers with which of the named environment variables the child can see."""
    conn.send_bytes(json.dumps({name: name in os.environ for name in names}).encode())
    conn.close()


def report_workdir(conn: AnswerChannel) -> None:
    """Writes a file in the child's working directory and answers with its location."""
    Path("scratch.txt").write_text("secret scratch", encoding="utf-8")
    answer = {
        "cwd": str(Path.cwd()),
        "tmpdir": os.environ.get("TMPDIR", ""),
        "temp": os.environ.get("TEMP", ""),
        "tmp": os.environ.get("TMP", ""),
        "file_exists": Path("scratch.txt").exists(),
    }
    conn.send_bytes(json.dumps(answer).encode())
    conn.close()


def relative_import_after_chdir(conn: AnswerChannel) -> None:
    """Imports a module of the worker only now, after the child has changed directory."""
    import importlib

    module = importlib.import_module("clearcut_worker.logs")
    conn.send_bytes(module.__name__.encode())
    conn.close()


def big_answer(conn: AnswerChannel, size: int) -> None:
    """Answers with `size` bytes."""
    conn.send_bytes(b"x" * size)
    conn.close()


def hang_after_writing(conn: AnswerChannel) -> None:
    """Leaves a file behind in its working directory and never answers."""
    Path("left-behind.txt").write_text("x", encoding="utf-8")
    time.sleep(120)


def write_marker_then_answer(conn: AnswerChannel, marker: str) -> None:
    """Proves it ran by creating the file at the given path."""
    Path(marker).write_text("ran", encoding="utf-8")
    conn.send_bytes(b"ran")
    conn.close()


def allocate_too_much(conn: AnswerChannel) -> None:
    """Tries to take two gigabytes at once."""
    try:
        block = bytearray(2 * 1024**3)
    except MemoryError:
        conn.send_bytes(b"memory error")
    else:
        conn.send_bytes(b"allocated %d" % len(block))
    conn.close()


def spin(conn: AnswerChannel) -> None:
    """Burns processor time for ever."""
    while True:
        pass


def report_limits(conn: AnswerChannel) -> None:
    """Answers with the soft limits the child is running under (Linux)."""
    import importlib

    resource = importlib.import_module("resource")
    names = ["RLIMIT_AS", "RLIMIT_CPU", "RLIMIT_NOFILE", "RLIMIT_CORE", "RLIMIT_FSIZE"]
    answer = {n: resource.getrlimit(getattr(resource, n))[0] for n in names}
    conn.send_bytes(json.dumps(answer).encode())
    conn.close()


def failing_limiter(limits: ChildLimits) -> list[str]:
    """A limiter that cannot do its job."""
    raise OSError(sys.platform)


def import_relative(conn: AnswerChannel) -> None:
    """Imports a module that is found only through a relative entry on the module path."""
    import importlib

    module = importlib.import_module("relmod")
    conn.send_bytes(str(module.VALUE).encode())
    conn.close()


def report_relative_path_entries(conn: AnswerChannel) -> None:
    """Answers with every entry on the module path that is not an absolute location."""
    relative = [entry for entry in sys.path if entry and not os.path.isabs(entry)]
    conn.send_bytes(json.dumps(relative).encode())
    conn.close()


def answer_late(conn: AnswerChannel, seconds: float) -> None:
    """Answers after a pause."""
    time.sleep(seconds)
    conn.send_bytes(b"late")
    conn.close()


def only_thirteen_cpu_seconds(limits: ChildLimits) -> list[str]:
    """A limiter that refuses to go on unless it was handed exactly 13 seconds of processor time."""
    if limits.cpu_seconds != 13:
        raise OSError(limits.cpu_seconds)
    return []


def report_initial_environment(conn: AnswerChannel, *values: str) -> None:
    """Answers with which of the given values appear in the environment the program started with.

    On Linux ``/proc/self/environ`` is that starting block; removing a variable
    from ``os.environ`` later does not change it.
    """
    initial = Path("/proc/self/environ").read_bytes().decode("utf-8", errors="replace")
    conn.send_bytes(json.dumps({value: value in initial for value in values}).encode())
    conn.close()
