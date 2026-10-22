# Written by Kone Tshivhinda & Claude
"""Inspecting in a child process that the parent can kill (the parser is hostile territory)."""

from __future__ import annotations

import inspect as stdlib_inspect
import json
import multiprocessing
import threading
import time
from pathlib import Path

import pytest

from clearcut_worker.app import create_app
from clearcut_worker.inspect import (
    Encrypted,
    Inspection,
    NotAPdf,
    TooManyPages,
    TooSlow,
    Unreadable,
)
from clearcut_worker.isolation import _child_main, inspect_isolated

from . import hang
from .conftest import make_encrypted_pdf, make_pdf


def no_children_left(wait: float = 5.0) -> bool:
    deadline = time.monotonic() + wait
    while time.monotonic() < deadline:
        if not multiprocessing.active_children():
            return True
        time.sleep(0.05)
    return False


def test_a_good_document_gives_the_same_answer_as_looking_in_process() -> None:
    result = inspect_isolated(make_pdf(["Invoice", None, "Total"]), 50, 30.0)
    assert result == Inspection(pages=3, pages_with_text=2)


def test_every_kind_of_refusal_survives_the_trip_back_from_the_child() -> None:
    with pytest.raises(NotAPdf):
        inspect_isolated(b"<html>nope</html>", 50, 30.0)
    with pytest.raises(Encrypted):
        inspect_isolated(make_encrypted_pdf(), 50, 30.0)
    with pytest.raises(Unreadable):
        inspect_isolated(b"%PDF-1.4\nSECRET garbage", 50, 30.0)
    with pytest.raises(TooManyPages) as caught:
        inspect_isolated(make_pdf(["a"] * 4), 3, 30.0)
    assert caught.value.max_pages == 3


def test_a_deadline_that_has_already_passed_is_too_slow() -> None:
    with pytest.raises(TooSlow):
        inspect_isolated(make_pdf(["a"]), 50, 0.0)


def test_a_parser_stuck_forever_is_killed_at_the_hard_limit() -> None:
    started = time.monotonic()
    with pytest.raises(TooSlow):
        inspect_isolated(make_pdf(["a"]), 50, 0.5, grace_seconds=0.5, target=hang.hang)
    assert time.monotonic() - started < 15, "it must not wait for the stuck parser"
    assert no_children_left(), "the stuck process must really be gone"


def test_a_parser_that_dies_hard_is_an_unreadable_document_not_a_dead_worker() -> None:
    with pytest.raises(Unreadable):
        inspect_isolated(make_pdf(["a"]), 50, 30.0, target=hang.crash)
    assert no_children_left()


def test_a_child_that_answers_nonsense_is_treated_as_unreadable() -> None:
    with pytest.raises(Unreadable) as caught:
        inspect_isolated(make_pdf(["a"]), 50, 30.0, target=hang.nonsense)
    assert "SECRET" not in str(caught.value)
    with pytest.raises(Unreadable):
        inspect_isolated(make_pdf(["a"]), 50, 30.0, target=hang.unknown_error)


def test_an_answer_of_the_wrong_shape_or_size_is_unreadable() -> None:
    for target in (hang.wrong_shape, hang.too_much):
        with pytest.raises(Unreadable):
            inspect_isolated(make_pdf(["a"]), 50, 30.0, target=target)
    assert no_children_left()


def test_a_compromised_child_cannot_run_code_in_the_parent(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    marker = tmp_path / "pwned"
    monkeypatch.setenv("CLEARCUT_TEST_MARKER", str(marker))
    with pytest.raises(Unreadable):
        inspect_isolated(make_pdf(["a"]), 50, 30.0, target=hang.malicious_pickle)
    assert not marker.exists(), "the parent unpickled something the child sent"
    assert no_children_left()


def test_several_inspections_at_once_do_not_interfere() -> None:
    results: dict[int, Inspection] = {}

    def run(n: int) -> None:
        results[n] = inspect_isolated(make_pdf(["x"] * n), 50, 30.0)

    threads = [threading.Thread(target=run, args=(n,)) for n in (1, 2, 3, 4)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    assert {n: r.pages for n, r in results.items()} == {1: 1, 2: 2, 3: 3, 4: 4}
    assert no_children_left()


def test_the_app_uses_the_isolated_inspector_unless_told_otherwise() -> None:
    default = stdlib_inspect.signature(create_app).parameters["inspector"].default
    assert default is inspect_isolated


def ask_child(answer: bytes) -> Inspection:
    """What the parent makes of a child that answers with exactly these bytes."""
    return inspect_isolated(answer, 50, 30.0, target=hang.echo)


@pytest.mark.parametrize(
    "answer",
    [
        b'["ok", 3, 2]',
        b'["ok", 0, 0]',
        b'["ok", 5, 5]',
    ],
)
def test_a_well_formed_answer_is_accepted(answer: bytes) -> None:
    result = ask_child(answer)
    assert (result.pages, result.pages_with_text) == tuple(json.loads(answer)[1:])


@pytest.mark.parametrize(
    "answer",
    [
        b"",
        b"null",
        b"{}",
        b'"ok"',
        b"[]",
        b'["ok"]',
        b'["ok", 3]',
        b'["ok", 3, 2, 1]',
        b'["ok", -1, 0]',
        b'["ok", 3, -1]',
        b'["ok", 2, 3]',
        b'["ok", true, true]',
        b'["ok", 1.0, 1]',
        b'["ok", "3", "2"]',
        b'["ok", null, 2]',
        b'["okay", 3, 2]',
        b'["error"]',
        b'["error", "NotAPdf", "extra"]',
        b'["error", 5]',
        b'["error", null]',
        b'["error", "notapdf"]',
        b'["error", "InspectError"]',
        b"[" + b" " * 300 + b'"error", "NotAPdf"]',
    ],
)
def test_any_other_answer_is_unreadable(answer: bytes) -> None:
    with pytest.raises(Unreadable):
        ask_child(answer)


@pytest.mark.parametrize(
    ("answer", "expected"),
    [
        (b'["error", "NotAPdf"]', NotAPdf),
        (b'["error", "Encrypted"]', Encrypted),
        (b'["error", "Unreadable"]', Unreadable),
        (b'["error", "TooSlow"]', TooSlow),
        (b'["error", "TooManyPages"]', TooManyPages),
    ],
)
def test_each_named_refusal_comes_back_as_itself(answer: bytes, expected: type[Exception]) -> None:
    with pytest.raises(expected):
        ask_child(answer)


# ---------- the child's own side, run in-process so every branch is reachable ----------


def run_child(data: bytes, max_pages: int = 50, timeout: float = 30.0) -> list[object]:
    receiver, sender = multiprocessing.get_context("spawn").Pipe(duplex=False)
    _child_main(sender, data, max_pages, timeout)
    answer: list[object] = json.loads(receiver.recv_bytes())
    receiver.close()
    return answer


def test_the_child_reports_what_it_found() -> None:
    assert run_child(make_pdf(["a", None, "c"])) == ["ok", 3, 2]


def test_the_child_names_each_refusal() -> None:
    assert run_child(b"<html/>") == ["error", "NotAPdf"]
    assert run_child(make_encrypted_pdf()) == ["error", "Encrypted"]
    assert run_child(b"%PDF-1.4\ngarbage") == ["error", "Unreadable"]
    assert run_child(make_pdf(["a"] * 4), max_pages=3) == ["error", "TooManyPages"]
    assert run_child(make_pdf(["a"]), timeout=0.0) == ["error", "TooSlow"]


def test_the_child_turns_any_surprise_into_unreadable_without_leaking_it(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    def boom(*args: object) -> Inspection:
        raise RuntimeError("SECRET parser blew up")

    monkeypatch.setattr("clearcut_worker.isolation.inspect_pdf", boom)
    answer = run_child(make_pdf(["a"]))
    assert answer == ["error", "Unreadable"]
    assert "SECRET" not in json.dumps(answer)


def test_an_answer_with_a_wrong_first_word_is_not_an_error_report() -> None:
    with pytest.raises(Unreadable):
        ask_child(b'["fail", "NotAPdf"]')
    with pytest.raises(Unreadable):
        ask_child(b'["ERROR", "NotAPdf"]')
