# Written by Kone Tshivhinda & Claude
"""Settings come from the environment, and bad ones stop the worker from starting."""

from __future__ import annotations

import pytest

from clearcut_worker.__main__ import main
from clearcut_worker.config import ConfigError, Settings

GOOD = {"CLEARCUT_WORKER_TOKEN": "k" * 32}


def test_defaults_are_the_agreed_ones() -> None:
    s = Settings.from_env(GOOD)
    assert s.token == "k" * 32
    assert s.max_request_bytes == 64 * 1024 * 1024
    assert s.max_pages == 500
    assert s.timeout_seconds == 30.0


def test_every_limit_can_be_set_from_the_environment() -> None:
    s = Settings.from_env(
        {
            **GOOD,
            "CLEARCUT_WORKER_MAX_BYTES": "1000",
            "CLEARCUT_WORKER_MAX_PAGES": "7",
            "CLEARCUT_WORKER_TIMEOUT_SECONDS": "2.5",
        }
    )
    assert (s.max_request_bytes, s.max_pages, s.timeout_seconds) == (1000, 7, 2.5)


@pytest.mark.parametrize("token", [None, "", "short", "k" * 31, " " * 40])
def test_a_missing_or_weak_token_is_refused(token: str | None) -> None:
    env = {} if token is None else {"CLEARCUT_WORKER_TOKEN": token}
    with pytest.raises(ConfigError, match="CLEARCUT_WORKER_TOKEN"):
        Settings.from_env(env)


def test_a_token_of_exactly_32_characters_is_enough() -> None:
    assert Settings.from_env({"CLEARCUT_WORKER_TOKEN": "x" * 32}).token == "x" * 32


@pytest.mark.parametrize(
    ("name", "value"),
    [
        ("CLEARCUT_WORKER_MAX_BYTES", "0"),
        ("CLEARCUT_WORKER_MAX_BYTES", "-5"),
        ("CLEARCUT_WORKER_MAX_BYTES", "lots"),
        ("CLEARCUT_WORKER_MAX_BYTES", "1.5"),
        ("CLEARCUT_WORKER_MAX_BYTES", ""),
        ("CLEARCUT_WORKER_MAX_PAGES", "0"),
        ("CLEARCUT_WORKER_MAX_PAGES", "-1"),
        ("CLEARCUT_WORKER_MAX_PAGES", "many"),
        ("CLEARCUT_WORKER_TIMEOUT_SECONDS", "0"),
        ("CLEARCUT_WORKER_TIMEOUT_SECONDS", "-1"),
        ("CLEARCUT_WORKER_TIMEOUT_SECONDS", "soon"),
        ("CLEARCUT_WORKER_TIMEOUT_SECONDS", "nan"),
        ("CLEARCUT_WORKER_TIMEOUT_SECONDS", "inf"),
    ],
)
def test_a_limit_that_cannot_work_is_refused_and_named(name: str, value: str) -> None:
    with pytest.raises(ConfigError, match=name):
        Settings.from_env({**GOOD, name: value})


def test_the_settings_never_show_the_token() -> None:
    s = Settings.from_env({"CLEARCUT_WORKER_TOKEN": "SECRET-" + "k" * 30})
    assert "SECRET" not in repr(s)
    assert "SECRET" not in str(s)


def test_the_error_for_a_bad_token_does_not_repeat_it() -> None:
    with pytest.raises(ConfigError) as caught:
        Settings.from_env({"CLEARCUT_WORKER_TOKEN": "SECRET"})
    assert "SECRET" not in str(caught.value)


def test_the_worker_refuses_to_start_without_a_token(capsys: pytest.CaptureFixture[str]) -> None:
    with pytest.raises(SystemExit) as caught:
        main({})
    assert caught.value.code == 2
    err = capsys.readouterr().err
    assert "CLEARCUT_WORKER_TOKEN" in err
