# Written by Kone Tshivhinda & Claude
"""A caller must not be able to forge or flood log lines."""

from __future__ import annotations

import logging

import pytest
from fastapi.testclient import TestClient

from clearcut_worker.app import create_app
from clearcut_worker.logs import safe_log_text

from .conftest import AUTH, make_pdf, settings


def worker_lines(caplog: pytest.LogCaptureFixture) -> list[str]:
    return [r.getMessage() for r in caplog.records if r.name == "clearcut_worker"]


def test_control_characters_are_escaped_not_printed() -> None:
    shown = safe_log_text("a\nb\r\tc\x00d\x1b[31mred")
    assert shown == "a\\nb\\r\\tc\\x00d\\x1b[31mred"
    assert all(ord(c) >= 32 for c in shown)


def test_non_ascii_text_is_escaped_too() -> None:
    assert safe_log_text("café") == "caf\\xe9"
    assert safe_log_text("\u2028line separator") == "\\u2028line separator"


def test_plain_text_is_left_alone() -> None:
    assert safe_log_text("/v1/inspect?x=1") == "/v1/inspect?x=1"


def test_a_path_with_an_encoded_newline_cannot_forge_a_log_line(
    client: TestClient, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.INFO)
    client.get("/x%0A2026-01-01 00:00:00 INFO POST /v1/inspect -> 200 (999 bytes, 1 ms)")
    lines = worker_lines(caplog)
    assert len(lines) == 1
    assert len(lines[0].splitlines()) == 1, "one request, one line"
    assert "\n" not in lines[0]
    assert "\\n2026-01-01" in lines[0], "the newline is visible, as an escape"
    assert "-> 404" in lines[0]


def test_other_control_characters_in_a_path_are_escaped(
    client: TestClient, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.INFO)
    client.get("/a%00b%1b%5b31mred%0D")
    (line,) = worker_lines(caplog)
    assert all(ord(c) >= 32 for c in line)
    assert "\\x00" in line
    assert "\\x1b" in line


def test_a_known_route_is_logged_by_its_template_not_the_raw_path(
    client: TestClient, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.INFO)
    client.post("/v1/inspect", content=make_pdf(["x"]), headers=AUTH)
    (line,) = worker_lines(caplog)
    assert line.startswith("POST /v1/inspect -> 200 (")


def test_a_very_long_unknown_path_is_cut_short_in_the_log(
    client: TestClient, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.INFO)
    client.get("/" + "a" * 5000)
    (line,) = worker_lines(caplog)
    assert len(line) < 400
    assert "aaaa" in line


def test_the_method_is_escaped_as_well(caplog: pytest.LogCaptureFixture) -> None:
    app = create_app(settings())
    caplog.set_level(logging.INFO)
    scope = {
        "type": "http",
        "asgi": {"version": "3.0"},
        "http_version": "1.1",
        "method": "GET\nFORGED",
        "scheme": "http",
        "path": "/health",
        "raw_path": b"/health",
        "query_string": b"",
        "headers": [],
        "client": ("testclient", 1),
        "server": ("testserver", 80),
    }
    sent: list[dict[str, object]] = []

    async def receive() -> dict[str, object]:
        return {"type": "http.request", "body": b"", "more_body": False}

    async def send(message: dict[str, object]) -> None:
        sent.append(message)

    import asyncio

    asyncio.run(app(scope, receive, send))  # type: ignore[arg-type]
    (line,) = worker_lines(caplog)
    assert "\n" not in line
    assert "GET\\nFORGED" in line
