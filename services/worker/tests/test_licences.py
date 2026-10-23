# Written by Kone Tshivhinda & Claude
"""The worker image may hold permissively licensed code only (ADR-0016).

A package licensed under the GPL, LGPL or AGPL, or under terms we cannot read,
must not slip into what we ship. These tests pin down the rules; the last one
runs them over the real runtime dependencies.
"""

from __future__ import annotations

from email.message import Message
from pathlib import Path

import pytest

from licences import (
    LicenceException,
    Problem,
    check,
    expression_allowed,
    licence_of,
    load_exceptions,
    runtime_closure,
)

# ---------- the rule for one licence expression ----------


@pytest.mark.parametrize(
    "expression",
    [
        "MIT",
        "MIT License",
        "BSD-3-Clause",
        "BSD License",
        "BSD-2-Clause",
        "Apache-2.0",
        "Apache Software License",
        "Apache License 2.0",
        "ISC",
        "ISC License (ISCL)",
        "PSF-2.0",
        "Python Software Foundation License",
        "HPND",
        "Historical Permission Notice and Disclaimer (HPND)",
        "Unlicense",
        "CC0-1.0",
        "0BSD",
        "Apache-2.0 OR BSD-2-Clause",
        "MIT AND BSD-3-Clause",
        "(MIT OR Apache-2.0) AND BSD-3-Clause",
        "MIT OR GPL-2.0-only",
        "Apache-2.0 WITH LLVM-exception",
        "mit",
    ],
)
def test_permissive_licences_are_allowed(expression: str) -> None:
    assert expression_allowed(expression), expression


@pytest.mark.parametrize(
    "expression",
    [
        "GPL-3.0-or-later",
        "GPL-2.0-only",
        "GPL-2.0+",
        "LGPL-2.1-only",
        "LGPL-3.0-or-later",
        "AGPL-3.0-only",
        "AGPL-3.0-or-later",
        "GNU General Public License v3 (GPLv3)",
        "GNU Affero General Public License v3",
        "GNU Lesser General Public License v2 or later (LGPLv2+)",
        "Mozilla Public License 2.0 (MPL 2.0)",
        "MPL-2.0",
        "SSPL-1.0",
        "BUSL-1.1",
        "CC-BY-NC-4.0",
        "Proprietary",
        "Commercial",
        "",
        "   ",
        "UNKNOWN",
        "BSD-3-Clause, Apache-2.0, dependency licenses",
        "MIT AND GPL-2.0-only",
        "MIT AND (Apache-2.0 OR LGPL-2.1-only) AND AGPL-3.0-only",
        "(MIT",
        "MIT OR",
        "AND MIT",
        "MIT MIT",
        "(MIT) MIT",
        "MIT (MIT)",
    ],
)
def test_everything_else_is_refused(expression: str) -> None:
    assert not expression_allowed(expression), expression


def test_an_or_needs_one_good_side_and_an_and_needs_both() -> None:
    assert expression_allowed("GPL-3.0-only OR MIT")
    assert not expression_allowed("GPL-3.0-only OR AGPL-3.0-only")
    assert expression_allowed("MIT AND Apache-2.0")
    assert not expression_allowed("MIT AND GPL-3.0-only")
    assert expression_allowed("MIT OR (GPL-3.0-only AND AGPL-3.0-only)")


# ---------- reading a package's own statement ----------


class FakeDist:
    """Just enough of an installed package: a name, metadata and requirements."""

    def __init__(
        self,
        name: str,
        *,
        expression: str | None = None,
        classifiers: tuple[str, ...] = (),
        licence: str | None = None,
        requires: tuple[str, ...] = (),
        version: str = "1.0",
    ) -> None:
        message = Message()
        message["Name"] = name
        message["Version"] = version
        if expression is not None:
            message["License-Expression"] = expression
        if licence is not None:
            message["License"] = licence
        for classifier in classifiers:
            message["Classifier"] = classifier
        self.metadata = message
        self.requires = list(requires) or None
        self.version = version


def test_the_license_expression_comes_first() -> None:
    dist = FakeDist(
        "a",
        expression="MIT",
        classifiers=("License :: OSI Approved :: BSD License",),
        licence="GPL",
    )
    assert licence_of(dist) == "MIT"


def test_classifiers_come_next_and_several_mean_all_of_them() -> None:
    one = FakeDist("a", classifiers=("License :: OSI Approved :: MIT License", "Topic :: Text"))
    assert licence_of(one) == "MIT License"
    two = FakeDist(
        "b",
        classifiers=(
            "License :: OSI Approved :: MIT License",
            "License :: OSI Approved :: GNU General Public License v3 (GPLv3)",
        ),
    )
    assert licence_of(two) == "MIT License AND GNU General Public License v3 (GPLv3)"
    assert not expression_allowed(licence_of(two))


def test_a_short_licence_field_is_used_and_a_long_one_is_not_believed() -> None:
    assert licence_of(FakeDist("a", licence="MIT")) == "MIT"
    long_text = "Permission is hereby granted, free of charge, to any person " * 10
    assert licence_of(FakeDist("b", licence=long_text)) == "UNKNOWN"
    assert licence_of(FakeDist("c")) == "UNKNOWN"
    assert licence_of(FakeDist("d", licence="MIT\nGPL-3.0-only")) == "UNKNOWN"


# ---------- the packages that are shipped ----------


def environment(*dists: FakeDist) -> dict[str, FakeDist]:
    return {d.metadata["Name"].lower(): d for d in dists}


def test_the_closure_follows_requirements_and_leaves_the_rest_out() -> None:
    env = environment(
        FakeDist("app", requires=("lib>=1", "other; extra == 'dev'")),
        FakeDist("lib", requires=("deep",)),
        FakeDist("deep"),
        FakeDist("other"),
        FakeDist("pytest"),
    )
    names = {d.metadata["Name"] for d in runtime_closure(["app"], env)}
    assert names == {"app", "lib", "deep"}, "extras and unrelated packages are not shipped"


def test_requirement_names_are_matched_the_way_pip_matches_them() -> None:
    env = {
        "app": FakeDist("app", requires=("My_Lib.Thing",)),
        "my-lib-thing": FakeDist("My_Lib.Thing"),
    }
    names = {d.metadata["Name"] for d in runtime_closure(["APP"], env)}
    assert names == {"app", "My_Lib.Thing"}


def test_a_requirement_for_another_platform_is_not_followed() -> None:
    env = environment(
        FakeDist("app", requires=("winonly; sys_platform == 'nonexistent-os'", "always")),
        FakeDist("winonly"),
        FakeDist("always"),
    )
    names = {d.metadata["Name"] for d in runtime_closure(["app"], env)}
    assert names == {"app", "always"}


def test_a_requirement_that_is_not_installed_is_reported_not_ignored() -> None:
    env = environment(FakeDist("app", requires=("missing-lib",)))
    with pytest.raises(LookupError) as caught:
        runtime_closure(["app"], env)
    assert "missing-lib" in str(caught.value)


def test_a_clean_set_of_packages_has_no_problems() -> None:
    env = environment(
        FakeDist("app", expression="Apache-2.0", requires=("lib",)),
        FakeDist("lib", expression="MIT"),
    )
    assert check(["app"], env, []) == []


def test_a_bad_licence_is_reported_with_the_package_and_what_it_says() -> None:
    env = environment(
        FakeDist("app", expression="MIT", requires=("lib",)),
        FakeDist("lib", expression="AGPL-3.0-only", version="2.5"),
    )
    problems = check(["app"], env, [])
    assert problems == [
        Problem(package="lib", version="2.5", licence="AGPL-3.0-only", kind="refused")
    ]
    assert "lib" in str(problems[0])
    assert "AGPL-3.0-only" in str(problems[0])


def test_an_unreadable_licence_is_reported_as_unknown() -> None:
    env = environment(FakeDist("app", expression="MIT", requires=("lib",)), FakeDist("lib"))
    problems = check(["app"], env, [])
    assert [(p.package, p.licence) for p in problems] == [("lib", "UNKNOWN")]


# ---------- exceptions ----------


def test_an_exception_with_a_reason_lets_exactly_that_wording_through() -> None:
    wording = "BSD-3-Clause, Apache-2.0, dependency licenses"
    env = environment(
        FakeDist("app", expression="MIT", requires=("pdf",)), FakeDist("pdf", licence=wording)
    )
    allowed = [
        LicenceException(
            package="pdf", licence=wording, reason="PDFium and what it bundles are permissive"
        )
    ]
    assert check(["app"], env, allowed) == []


def test_an_exception_stops_working_if_the_package_changes_its_wording() -> None:
    env = environment(
        FakeDist("app", expression="MIT", requires=("pdf",)),
        FakeDist("pdf", licence="GPL-3.0-only"),
    )
    allowed = [LicenceException(package="pdf", licence="BSD-3-Clause", reason="was fine")]
    problems = check(["app"], env, allowed)
    assert [(p.package, p.kind) for p in problems] == [("pdf", "refused")]


def test_an_exception_for_a_package_that_is_not_shipped_is_reported_as_stale() -> None:
    env = environment(FakeDist("app", expression="MIT"))
    allowed = [LicenceException(package="gone", licence="whatever", reason="old")]
    problems = check(["app"], env, allowed)
    assert [(p.package, p.kind) for p in problems] == [("gone", "stale exception")]


def test_the_exceptions_file_is_read_and_must_give_a_reason(tmp_path: Path) -> None:
    good = tmp_path / "good.toml"
    good.write_text(
        '[[exception]]\npackage = "pdf"\nlicence = "A, B"\nreason = "because"\n', encoding="utf-8"
    )
    assert load_exceptions(good) == [
        LicenceException(package="pdf", licence="A, B", reason="because")
    ]

    for text in [
        '[[exception]]\npackage = "pdf"\nlicence = "A"\n',
        '[[exception]]\npackage = "pdf"\nlicence = "A"\nreason = "   "\n',
        '[[exception]]\nlicence = "A"\nreason = "x"\n',
        "not toml at all [[[",
    ]:
        bad = tmp_path / "bad.toml"
        bad.write_text(text, encoding="utf-8")
        with pytest.raises(ValueError, match="licence exceptions"):
            load_exceptions(bad)


def test_a_missing_exceptions_file_means_no_exceptions(tmp_path: Path) -> None:
    assert load_exceptions(tmp_path / "nothing.toml") == []


# ---------- the real thing ----------


def test_everything_the_worker_ships_is_permissively_licensed() -> None:
    from importlib import metadata

    from licences import EXCEPTIONS_FILE, REQUIREMENTS_FILE, read_requirement_names

    installed = {
        d.metadata["Name"].lower().replace("_", "-").replace(".", "-"): d
        for d in metadata.distributions()
    }
    problems = check(
        read_requirement_names(REQUIREMENTS_FILE), installed, load_exceptions(EXCEPTIONS_FILE)
    )
    assert problems == [], "\n".join(str(p) for p in problems)
