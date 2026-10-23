# Written by Kone Tshivhinda & Claude
"""What the child process is allowed to take (ADR-0016): memory, processor time, files."""

from __future__ import annotations

import pytest

from clearcut_worker.limits import (
    ChildLimits,
    LimitError,
    apply_limits,
    scrub_environment,
)


class FakeResource:
    """Stands in for the `resource` module, so the rules are tested on any platform."""

    RLIMIT_AS = 9
    RLIMIT_CPU = 0
    RLIMIT_NOFILE = 7
    RLIMIT_CORE = 4
    RLIMIT_FSIZE = 1
    RLIM_INFINITY = -1

    def __init__(self, hard: dict[int, int] | None = None, fail_on: int | None = None) -> None:
        self.hard = hard or {}
        self.fail_on = fail_on
        self.calls: list[tuple[int, tuple[int, int]]] = []

    def getrlimit(self, which: int) -> tuple[int, int]:
        hard = self.hard.get(which, self.RLIM_INFINITY)
        return (hard, hard)

    def setrlimit(self, which: int, values: tuple[int, int]) -> None:
        if which == self.fail_on:
            raise OSError("not permitted")
        self.calls.append((which, values))


def test_the_limits_must_be_positive_whole_numbers() -> None:
    ChildLimits()
    for field in ("address_space_bytes", "cpu_seconds", "open_files", "file_size_bytes"):
        for bad in (0, -1):
            with pytest.raises(ValueError, match="limit"):
                ChildLimits(**{field: bad})


def test_every_limit_is_set_and_core_dumps_are_switched_off() -> None:
    fake = FakeResource()
    limits = ChildLimits(
        address_space_bytes=1000, cpu_seconds=7, open_files=33, file_size_bytes=555
    )
    applied = apply_limits(limits, resource_module=fake)
    assert dict(fake.calls) == {
        FakeResource.RLIMIT_AS: (1000, 1000),
        FakeResource.RLIMIT_CPU: (7, 7),
        FakeResource.RLIMIT_NOFILE: (33, 33),
        FakeResource.RLIMIT_FSIZE: (555, 555),
        FakeResource.RLIMIT_CORE: (0, 0),
    }
    assert sorted(applied) == ["core", "cpu", "file_size", "memory", "open_files"]


def test_a_limit_is_never_raised_above_what_the_system_already_allows() -> None:
    fake = FakeResource(hard={FakeResource.RLIMIT_AS: 500, FakeResource.RLIMIT_NOFILE: 10})
    apply_limits(
        ChildLimits(address_space_bytes=1000, cpu_seconds=7, open_files=33, file_size_bytes=555),
        resource_module=fake,
    )
    calls = dict(fake.calls)
    assert calls[FakeResource.RLIMIT_AS] == (500, 500)
    assert calls[FakeResource.RLIMIT_NOFILE] == (10, 10)
    assert calls[FakeResource.RLIMIT_CPU] == (7, 7)


def test_a_limit_that_cannot_be_set_is_an_error_not_a_shrug() -> None:
    fake = FakeResource(fail_on=FakeResource.RLIMIT_AS)
    with pytest.raises(LimitError) as caught:
        apply_limits(ChildLimits(), resource_module=fake)
    assert "memory" in str(caught.value)


def test_without_the_resource_module_nothing_is_set_and_nothing_fails() -> None:
    # Windows has no `resource`; the container supplies the limits there.
    assert apply_limits(ChildLimits(), resource_module=None) == []


# ---------- the environment ----------


def test_secrets_in_the_environment_are_removed_and_the_basics_stay() -> None:
    env = {
        "CLEARCUT_WORKER_TOKEN": "placeholder-token",
        "ANTHROPIC_API_KEY": "placeholder-key",
        "AWS_SECRET_ACCESS_KEY": "placeholder",
        "DATABASE_URL": "postgres://placeholder",
        "HOME": "/home/worker",
        "PATH": "/usr/bin",
        "SYSTEMROOT": "C:\\Windows",
        "TMPDIR": "scratch",
        "TESSDATA_PREFIX": "/usr/share/tessdata",
        "LANG": "C.UTF-8",
    }
    removed = scrub_environment(env)
    assert sorted(env) == ["LANG", "PATH", "SYSTEMROOT", "TESSDATA_PREFIX", "TMPDIR"]
    assert sorted(removed) == [
        "ANTHROPIC_API_KEY",
        "AWS_SECRET_ACCESS_KEY",
        "CLEARCUT_WORKER_TOKEN",
        "DATABASE_URL",
        "HOME",
    ]


def test_scrubbing_reports_names_only_never_values() -> None:
    env = {"CLEARCUT_WORKER_TOKEN": "placeholder-token"}
    assert "placeholder" not in repr(scrub_environment(env))


def test_scrubbing_an_environment_with_nothing_to_remove_removes_nothing() -> None:
    env = {"PATH": "/usr/bin"}
    assert scrub_environment(env) == []
    assert env == {"PATH": "/usr/bin"}


def test_a_limit_the_platform_does_not_support_is_skipped_but_the_rest_are_set() -> None:
    class NoAddressSpace(FakeResource):
        def setrlimit(self, which: int, values: tuple[int, int]) -> None:
            if which == self.RLIMIT_AS:
                raise ValueError("not supported here")
            super().setrlimit(which, values)

    fake = NoAddressSpace()
    applied = apply_limits(ChildLimits(), resource_module=fake)
    assert "memory" not in applied
    assert sorted(applied) == ["core", "cpu", "file_size", "open_files"]


def test_a_limit_the_platform_does_not_have_is_skipped() -> None:
    class NoFileSize(FakeResource):
        RLIMIT_FSIZE = None  # type: ignore[assignment]

    applied = apply_limits(ChildLimits(), resource_module=NoFileSize())
    assert "file_size" not in applied
    assert "memory" in applied


def test_environment_names_are_compared_without_regard_to_case() -> None:
    env = {"Path": "x", "path_extra_secret": "y", "Tmpdir": "z"}
    removed = scrub_environment(env)
    assert sorted(env) == ["Path", "Tmpdir"]
    assert removed == ["path_extra_secret"]
