# Written by Kone Tshivhinda & Claude
"""Logging for the running worker.

uvicorn configures only its own loggers, so without this the worker's request
lines (method, path, status, size, time) would never appear. Lines go to stderr,
which is where a container's log collector looks. They never carry file content
or the token; the app and its tests keep to that.
"""

from __future__ import annotations

import logging


def safe_log_text(value: str) -> str:
    """Text that came from a caller, made safe to put in a log line.

    Control characters (a newline, say, that would start a forged log line) and
    anything outside plain ASCII are written as escapes, so one request is always
    exactly one line.
    """
    return value.encode("unicode_escape").decode("ascii")


def configure_logging() -> None:
    """Shows the worker's INFO lines on stderr. Safe to call more than once."""
    logger = logging.getLogger("clearcut_worker")
    logger.setLevel(logging.INFO)
    if not logger.handlers:
        handler = logging.StreamHandler()  # stderr
        handler.setFormatter(logging.Formatter("%(asctime)s %(levelname)s %(message)s"))
        logger.addHandler(handler)
