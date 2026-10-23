# Written by Kone Tshivhinda & Claude
"""Settings, read from the environment. Bad settings stop the worker from starting."""

from __future__ import annotations

import math
import re
from collections.abc import Mapping
from dataclasses import dataclass, field

_MIN_TOKEN_CHARS = 32
_VISIBLE_ASCII = re.compile(r"^[\x21-\x7e]+$")
_WHOLE_NUMBER = re.compile(r"^[0-9]+$")


class ConfigError(Exception):
    """A setting is missing or cannot work. The message names it, never its value."""


def _whole_number(env: Mapping[str, str], name: str, default: int) -> int:
    raw = env.get(name)
    if raw is None:
        return default
    if not _WHOLE_NUMBER.match(raw) or int(raw) == 0:
        raise ConfigError(f"{name} must be a whole number greater than zero")
    return int(raw)


def _seconds(env: Mapping[str, str], name: str, default: float) -> float:
    raw = env.get(name)
    if raw is None:
        return default
    try:
        value = float(raw)
    except ValueError:
        value = math.nan
    if not math.isfinite(value) or value <= 0:
        raise ConfigError(f"{name} must be a number of seconds greater than zero")
    return value


_MAX_PATH_CHARS = 4096


def _path(env: Mapping[str, str], name: str) -> str | None:
    """A program or directory path: no spaces at either end and no control characters."""
    raw = env.get(name)
    if raw is None:
        return None
    if (
        not raw
        or raw != raw.strip()
        or len(raw) > _MAX_PATH_CHARS
        or any(ord(c) < 32 or ord(c) == 127 for c in raw)
    ):
        raise ConfigError(f"{name} must be a path with no control characters or spaces at the ends")
    return raw


@dataclass(frozen=True)
class Settings:
    # The shared secret the Rust service presents. Never shown (not in repr, so
    # not in logs or tracebacks either).
    token: str = field(repr=False)
    max_request_bytes: int
    max_pages: int
    timeout_seconds: float
    # OCR is off unless the program is named. Its language data may be somewhere else.
    tesseract: str | None = None
    tessdata_dir: str | None = None

    @classmethod
    def from_env(cls, env: Mapping[str, str]) -> Settings:
        token = env.get("CLEARCUT_WORKER_TOKEN", "")
        if len(token) < _MIN_TOKEN_CHARS or not _VISIBLE_ASCII.match(token):
            raise ConfigError(
                "CLEARCUT_WORKER_TOKEN must be set to a secret of at least "
                f"{_MIN_TOKEN_CHARS} visible characters"
            )
        tesseract = _path(env, "CLEARCUT_WORKER_TESSERACT")
        tessdata_dir = _path(env, "CLEARCUT_WORKER_TESSDATA")
        if tessdata_dir is not None and tesseract is None:
            raise ConfigError("CLEARCUT_WORKER_TESSDATA is set without CLEARCUT_WORKER_TESSERACT")
        return cls(
            token=token,
            max_request_bytes=_whole_number(env, "CLEARCUT_WORKER_MAX_BYTES", 64 * 1024 * 1024),
            max_pages=_whole_number(env, "CLEARCUT_WORKER_MAX_PAGES", 500),
            timeout_seconds=_seconds(env, "CLEARCUT_WORKER_TIMEOUT_SECONDS", 30.0),
            tesseract=tesseract,
            tessdata_dir=tessdata_dir,
        )
