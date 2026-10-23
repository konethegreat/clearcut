# Written by Kone Tshivhinda & Claude
"""Starting the worker: logging, host, port."""

from __future__ import annotations

import logging
from collections.abc import Iterator
from typing import Any

import pytest

from clearcut_worker.__main__ import main
from clearcut_worker.logs import configure_logging

TOKEN_ENV = {"CLEARCUT_WORKER_TOKEN": "k" * 40}


@pytest.fixture
def clean_logger() -> Iterator[logging.Logger]:
    logger = logging.getLogger("clearcut_worker")
    saved_handlers, saved_level = logger.handlers[:], logger.level
    logger.handlers.clear()
    logger.setLevel(logging.NOTSET)
    yield logger
    logger.handlers[:] = saved_handlers
    logger.setLevel(saved_level)


def test_logging_shows_info_but_not_debug(clean_logger: logging.Logger) -> None:
    configure_logging()
    assert clean_logger.isEnabledFor(logging.INFO)
    assert not clean_logger.isEnabledFor(logging.DEBUG)


def test_configuring_twice_does_not_double_every_line(clean_logger: logging.Logger) -> None:
    configure_logging()
    configure_logging()
    assert len(clean_logger.handlers) == 1


def test_a_log_line_has_a_time_a_level_and_the_message(clean_logger: logging.Logger) -> None:
    configure_logging()
    record = logging.LogRecord(
        "clearcut_worker", logging.INFO, __file__, 1, "POST /v1/inspect", None, None
    )
    line = clean_logger.handlers[0].format(record)
    assert "INFO" in line
    assert "POST /v1/inspect" in line
    assert line.index("INFO") > 0, "a timestamp comes first"


class FakeRun:
    """Stands in for uvicorn.run so no server starts."""

    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []

    def __call__(self, app: object, **options: Any) -> None:
        self.calls.append({"app": app, **options})


@pytest.fixture
def fake_run(monkeypatch: pytest.MonkeyPatch) -> FakeRun:
    fake = FakeRun()
    monkeypatch.setattr("clearcut_worker.__main__.uvicorn.run", fake)
    return fake


def test_it_listens_on_loopback_port_8090_by_default(
    fake_run: FakeRun, clean_logger: logging.Logger
) -> None:
    main(TOKEN_ENV)
    (call,) = fake_run.calls
    assert (call["host"], call["port"]) == ("127.0.0.1", 8090)
    assert call["access_log"] is False
    assert clean_logger.handlers, "logging was set up before serving"


def test_host_and_port_can_be_set(fake_run: FakeRun, clean_logger: logging.Logger) -> None:
    main({**TOKEN_ENV, "CLEARCUT_WORKER_HOST": "0.0.0.0", "CLEARCUT_WORKER_PORT": "9000"})  # noqa: S104
    (call,) = fake_run.calls
    assert (call["host"], call["port"]) == ("0.0.0.0", 9000)  # noqa: S104


@pytest.mark.parametrize("port", ["", "http", "-1", "65536", "99999", "0", "80.5", "00000000"])
def test_a_port_that_cannot_work_stops_the_start(
    fake_run: FakeRun, port: str, capsys: pytest.CaptureFixture[str]
) -> None:
    with pytest.raises(SystemExit) as caught:
        main({**TOKEN_ENV, "CLEARCUT_WORKER_PORT": port})
    assert caught.value.code == 2
    assert "CLEARCUT_WORKER_PORT" in capsys.readouterr().err
    assert fake_run.calls == []


def test_a_bad_setting_never_starts_a_server(fake_run: FakeRun) -> None:
    with pytest.raises(SystemExit):
        main({"CLEARCUT_WORKER_TOKEN": "short"})
    assert fake_run.calls == []


@pytest.mark.parametrize("port", ["1", "80", "65535"])
def test_the_edges_of_the_port_range_are_allowed(
    fake_run: FakeRun, clean_logger: logging.Logger, port: str
) -> None:
    main({**TOKEN_ENV, "CLEARCUT_WORKER_PORT": port})
    assert fake_run.calls[0]["port"] == int(port)


# ---------- OCR is started only when it can be used ----------


def capabilities_of(call: dict[str, Any]) -> list[str]:
    from fastapi.testclient import TestClient

    return list(TestClient(call["app"]).get("/health").json()["capabilities"])


def test_without_a_named_program_ocr_is_off_and_nothing_is_looked_for(
    fake_run: FakeRun, clean_logger: logging.Logger, monkeypatch: pytest.MonkeyPatch
) -> None:
    def must_not_look(*args: object) -> None:
        raise AssertionError("looked for a program that was not asked for")

    monkeypatch.setattr("clearcut_worker.__main__.find_engine", must_not_look)
    main(TOKEN_ENV)
    assert capabilities_of(fake_run.calls[0]) == ["inspect", "text", "render"]


def test_a_named_program_that_works_turns_ocr_on(
    fake_run: FakeRun, clean_logger: logging.Logger, monkeypatch: pytest.MonkeyPatch
) -> None:
    from clearcut_worker.ocr import OcrEngine

    asked: list[tuple[object, object]] = []

    def found(program: str | None, data: str | None) -> OcrEngine:
        asked.append((program, data))
        return OcrEngine((program or "",), data)

    monkeypatch.setattr("clearcut_worker.__main__.find_engine", found)
    main({**TOKEN_ENV, "CLEARCUT_WORKER_TESSERACT": "/opt/t", "CLEARCUT_WORKER_TESSDATA": "/opt/d"})
    assert asked == [("/opt/t", "/opt/d")]
    assert capabilities_of(fake_run.calls[0]) == ["inspect", "text", "render", "ocr"]


def test_a_named_program_that_cannot_be_used_stops_the_worker_from_starting(
    fake_run: FakeRun,
    clean_logger: logging.Logger,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setattr("clearcut_worker.__main__.find_engine", lambda program, data: None)
    with pytest.raises(SystemExit) as caught:
        main({**TOKEN_ENV, "CLEARCUT_WORKER_TESSERACT": "/opt/t"})
    assert caught.value.code == 2
    error = capsys.readouterr().err
    assert "CLEARCUT_WORKER_TESSERACT" in error
    assert "English and Afrikaans" in error
    assert fake_run.calls == []
