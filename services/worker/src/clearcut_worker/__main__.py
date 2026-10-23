# Written by Kone Tshivhinda & Claude
"""Starts the worker: ``python -m clearcut_worker`` (with ``PYTHONPATH=src`` from a checkout)."""

from __future__ import annotations

import os
import re
import sys
from collections.abc import Mapping

import uvicorn

from .app import create_app
from .config import ConfigError, Settings
from .logs import configure_logging
from .ocr import OcrEngine, find_engine

_PORT = re.compile(r"^[0-9]{1,5}$")


def _port(env: Mapping[str, str]) -> int:
    raw = env.get("CLEARCUT_WORKER_PORT", "8090")
    if not _PORT.match(raw) or not 1 <= int(raw) <= 65535:
        raise ConfigError("CLEARCUT_WORKER_PORT must be a port number from 1 to 65535")
    return int(raw)


def main(environ: Mapping[str, str] | None = None) -> None:
    env = os.environ if environ is None else environ
    try:
        settings = Settings.from_env(env)
        port = _port(env)
    except ConfigError as error:
        # The message names the setting; it never contains a secret.
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(2) from None
    configure_logging()
    engine: OcrEngine | None = None
    if settings.tesseract is not None:
        # Named on purpose, so a program that cannot be used is a mistake to fix now,
        # not a surprise at the first scanned page.
        engine = find_engine(settings.tesseract, settings.tessdata_dir)
        if engine is None:
            print(
                "error: CLEARCUT_WORKER_TESSERACT names a program that cannot be used: it must "
                "start, and have both English and Afrikaans language data",
                file=sys.stderr,
            )
            raise SystemExit(2)
    uvicorn.run(
        create_app(settings, ocr_engine=engine),
        # Loopback unless told otherwise: the worker is for the Rust service only.
        host=env.get("CLEARCUT_WORKER_HOST", "127.0.0.1"),
        port=port,
        log_level="info",
        access_log=False,
    )


if __name__ == "__main__":
    main()
