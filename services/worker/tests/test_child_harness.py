# Written by Kone Tshivhinda & Claude
"""The harness every worker child runs in (ADR-0016): scrubbed, boxed in and cleaned up after."""

from __future__ import annotations

import json
import sys
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest

from clearcut_worker.inspect import TooSlow, Unreadable
from clearcut_worker.isolation import run_in_child
from clearcut_worker.limits import ChildLimits

from . import children
from .test_isolation import no_children_left

LINUX_ONLY = pytest.mark.skipif(sys.platform == "win32", reason="needs POSIX resource limits")


def run(target: Callable[..., None], *args: object, **options: Any) -> bytes:
    settings: dict[str, Any] = {"timeout_seconds": 30.0, "max_answer_bytes": 1024}
    settings.update(options)
    return run_in_child(target, args, **settings)


# ---------- answers ----------


def test_an_answer_larger_than_the_old_tiny_limit_is_fine_when_the_caller_allows_it() -> None:
    answer = run(children.big_answer, 1_000_000, max_answer_bytes=2_000_000)
    assert len(answer) == 1_000_000
    assert no_children_left()


def test_an_answer_over_the_callers_limit_is_unreadable() -> None:
    with pytest.raises(Unreadable):
        run(children.big_answer, 600_000, max_answer_bytes=500_000)
    assert no_children_left()


def test_an_answer_exactly_at_the_limit_is_accepted_and_one_over_is_not() -> None:
    assert len(run(children.big_answer, 4096, max_answer_bytes=4096)) == 4096
    with pytest.raises(Unreadable):
        run(children.big_answer, 4097, max_answer_bytes=4096)


# ---------- environment ----------


def test_the_child_cannot_see_the_workers_secrets(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("CLEARCUT_WORKER_TOKEN", "t" * 40)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "placeholder-key")
    answer = json.loads(
        run(children.report_environment, "CLEARCUT_WORKER_TOKEN", "ANTHROPIC_API_KEY")
    )
    assert answer == {"CLEARCUT_WORKER_TOKEN": False, "ANTHROPIC_API_KEY": False}


# ---------- the working directory ----------


def test_the_child_works_in_a_private_directory_that_is_removed_afterwards(tmp_path: Path) -> None:
    answer = json.loads(run(children.report_workdir, workdir_parent=tmp_path))
    assert answer["file_exists"] is True
    assert Path(answer["cwd"]).parent.resolve() == tmp_path.resolve()
    for name in ("tmpdir", "temp", "tmp"):
        assert Path(answer[name]).resolve() == Path(answer["cwd"]).resolve(), name
    assert list(tmp_path.iterdir()) == [], "the scratch directory must be gone"


def test_the_directory_is_removed_even_when_the_child_is_killed(tmp_path: Path) -> None:
    started = time.monotonic()
    with pytest.raises(TooSlow):
        run(
            children.hang_after_writing,
            timeout_seconds=0.5,
            grace_seconds=0.5,
            workdir_parent=tmp_path,
        )
    assert time.monotonic() - started < 15
    assert list(tmp_path.iterdir()) == []
    assert no_children_left()


def test_the_directory_is_removed_when_the_child_crashes(tmp_path: Path) -> None:
    from . import hang

    with pytest.raises(Unreadable):
        run(hang.crash, b"", 1, 1.0, workdir_parent=tmp_path)
    assert list(tmp_path.iterdir()) == []


def test_modules_can_still_be_imported_after_the_child_changes_directory() -> None:
    assert run(children.relative_import_after_chdir) == b"clearcut_worker.logs"


# ---------- limits ----------


def test_a_child_whose_limits_cannot_be_set_never_touches_the_document(tmp_path: Path) -> None:
    marker = tmp_path / "ran"
    with pytest.raises(Unreadable):
        run(
            children.write_marker_then_answer,
            str(marker),
            limits=ChildLimits(),
            limiter=children.failing_limiter,
        )
    assert not marker.exists()
    assert no_children_left()


@LINUX_ONLY
def test_the_child_really_runs_under_the_limits() -> None:
    limits = ChildLimits(
        address_space_bytes=1024**3, cpu_seconds=50, open_files=48, file_size_bytes=10_000_000
    )
    answer = json.loads(run(children.report_limits, limits=limits))
    assert answer["RLIMIT_AS"] == 1024**3
    assert answer["RLIMIT_NOFILE"] == 48
    assert answer["RLIMIT_FSIZE"] == 10_000_000
    assert answer["RLIMIT_CORE"] == 0
    assert 1 <= answer["RLIMIT_CPU"] <= 50


@LINUX_ONLY
def test_a_child_that_tries_to_take_too_much_memory_is_stopped() -> None:
    limits = ChildLimits(address_space_bytes=512 * 1024**2)
    assert run(children.allocate_too_much, limits=limits) == b"memory error"
    assert no_children_left()


@LINUX_ONLY
def test_a_child_that_burns_processor_time_is_stopped_by_the_limit() -> None:
    started = time.monotonic()
    with pytest.raises(Unreadable):
        run(children.spin, limits=ChildLimits(cpu_seconds=1), timeout_seconds=60.0)
    assert time.monotonic() - started < 20, "the processor limit, not the wall clock, ended it"
    assert no_children_left()


def test_a_fractional_wait_is_rounded_up_not_down() -> None:
    from clearcut_worker.isolation import effective_limits

    limits = ChildLimits(cpu_seconds=600)
    assert effective_limits(limits, timeout_seconds=10.5, grace_seconds=2.0).cpu_seconds == 14


def test_the_processor_limit_never_exceeds_the_time_the_parent_will_wait() -> None:
    from clearcut_worker.isolation import effective_limits

    base = ChildLimits(cpu_seconds=600)
    assert effective_limits(base, timeout_seconds=10.0, grace_seconds=2.0).cpu_seconds == 13
    small = ChildLimits(cpu_seconds=2)
    assert effective_limits(small, timeout_seconds=10.0, grace_seconds=2.0).cpu_seconds == 2


def test_no_relative_entry_is_left_on_the_childs_module_path(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.chdir(tmp_path)
    monkeypatch.syspath_prepend(".")
    assert json.loads(run(children.report_relative_path_entries)) == []


def test_an_answer_that_arrives_during_the_grace_period_is_accepted() -> None:
    answer = run(children.answer_late, 1.5, timeout_seconds=0.2, grace_seconds=8.0)
    assert answer == b"late"


def test_the_child_is_given_the_trimmed_processor_limit_not_the_raw_one() -> None:
    answer = run(
        children.big_answer,
        10,
        limits=ChildLimits(cpu_seconds=600),
        limiter=children.only_thirteen_cpu_seconds,
        timeout_seconds=10.0,
        grace_seconds=2.0,
    )
    assert answer == b"x" * 10


def test_the_inspector_runs_its_child_under_limits_unless_told_otherwise() -> None:
    import inspect as stdlib

    from clearcut_worker.isolation import DEFAULT_LIMITS, inspect_isolated

    default = stdlib.signature(inspect_isolated).parameters["limits"].default
    assert default is DEFAULT_LIMITS


# ---------- the starting environment ----------


@LINUX_ONLY
def test_the_childs_starting_environment_never_held_the_secrets(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    # Taking a variable out of os.environ after the child has started leaves it in the
    # block the program was started with, where /proc/self/environ shows it to anything
    # that has taken the parser over. So the child must be started without it.
    monkeypatch.setenv("CLEARCUT_WORKER_TOKEN", "placeholder-token-value")
    answer = json.loads(run(children.report_initial_environment, "placeholder-token-value"))
    assert answer == {"placeholder-token-value": False}


def test_the_environment_is_clean_only_while_a_child_starts_and_then_restored(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import os

    from clearcut_worker.isolation import clean_environment_for_start

    monkeypatch.setenv("CLEARCUT_WORKER_TOKEN", "placeholder-token-value")
    monkeypatch.setenv("SOMETHING_ELSE", "other")
    with clean_environment_for_start():
        assert "CLEARCUT_WORKER_TOKEN" not in os.environ
        assert "SOMETHING_ELSE" not in os.environ
        assert "PATH" in os.environ or "Path" in os.environ
    assert os.environ["CLEARCUT_WORKER_TOKEN"] == "placeholder-token-value"
    assert os.environ["SOMETHING_ELSE"] == "other"


def test_the_environment_is_restored_even_when_starting_fails(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import os

    from clearcut_worker.isolation import clean_environment_for_start

    monkeypatch.setenv("CLEARCUT_WORKER_TOKEN", "placeholder-token-value")
    with pytest.raises(RuntimeError), clean_environment_for_start():
        raise RuntimeError
    assert os.environ["CLEARCUT_WORKER_TOKEN"] == "placeholder-token-value"


def test_several_children_starting_at_once_leave_the_environment_as_it_was(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import os
    import threading

    monkeypatch.setenv("CLEARCUT_WORKER_TOKEN", "placeholder-token-value")
    failures: list[BaseException] = []

    def start_one() -> None:
        try:
            assert run(children.big_answer, 5) == b"x" * 5
        except BaseException as error:
            failures.append(error)

    threads = [threading.Thread(target=start_one) for _ in range(6)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    assert failures == []
    assert os.environ["CLEARCUT_WORKER_TOKEN"] == "placeholder-token-value"
