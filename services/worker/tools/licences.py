# Written by Kone Tshivhinda & Claude
"""Keeps the worker image permissively licensed (ADR-0016).

ClearCut is Apache-2.0 and meant to be used freely. Anything in the worker that
is GPL, LGPL or AGPL licensed (or licensed on terms nobody can read) would put
conditions on everyone who deploys it, so such a package must never be added by
accident. This tool reads the licence each installed package states about
itself, follows the packages the worker actually ships (``requirements.txt``
and what those need, not the test tools), and refuses anything outside a short
allowlist.

A package whose licence cannot be read by the rules (for example a free-text
field naming several licences) needs an entry in ``licence-exceptions.toml``
with the exact wording and a reason. The entry stops working if the wording
changes, and it is reported when the package is no longer shipped, so the file
cannot go stale.

Run it with ``python tools/licences.py`` from ``services/worker``.
"""

from __future__ import annotations

import re
import sys
import tomllib
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

from packaging.requirements import InvalidRequirement, Requirement

HERE = Path(__file__).resolve().parent
REQUIREMENTS_FILE = HERE.parent / "requirements.txt"
EXCEPTIONS_FILE = HERE.parent / "licence-exceptions.toml"

# Licence ids we accept, lower-cased. Everything here lets anyone use, change
# and redistribute the code, commercially or not, keeping only a notice.
_ALLOWED = frozenset(
    {
        "mit",
        "mit-0",
        "bsd-2-clause",
        "bsd-3-clause",
        "apache-2.0",
        "isc",
        "psf-2.0",
        "python-2.0",
        "hpnd",
        "unlicense",
        "cc0-1.0",
        "0bsd",
        "zlib",
    }
)

# The other ways packages write the same licences, lower-cased, mapped to an id.
_ALIASES = {
    "mit license": "mit",
    "bsd license": "bsd-3-clause",
    "apache software license": "apache-2.0",
    "apache license 2.0": "apache-2.0",
    "apache license, version 2.0": "apache-2.0",
    "apache 2.0": "apache-2.0",
    "isc license (iscl)": "isc",
    "python software foundation license": "psf-2.0",
    "historical permission notice and disclaimer (hpnd)": "hpnd",
    "the unlicense (unlicense)": "unlicense",
    "cc0 1.0 universal (cc0 1.0) public domain dedication": "cc0-1.0",
    "zlib/libpng license": "zlib",
}

# A licence field longer than this is a licence text, not a name.
_MAX_FIELD_CHARS = 100

_TOKEN = re.compile(r"\(|\)|\s+AND\s+|\s+OR\s+|\s+WITH\s+")


class _Dist(Protocol):
    """What this tool needs of an installed package."""

    @property
    def metadata(self) -> Any: ...

    @property
    def requires(self) -> list[str] | None: ...

    @property
    def version(self) -> str: ...


@dataclass(frozen=True)
class LicenceException:
    package: str
    licence: str
    reason: str


@dataclass(frozen=True)
class Problem:
    package: str
    version: str
    licence: str
    kind: str

    def __str__(self) -> str:
        if self.kind == "stale exception":
            return f"{self.package}: the licence exception is stale (the package is not shipped)"
        return (
            f"{self.package} {self.version}: licence {self.licence!r} is not allowed ({self.kind})"
        )


# ----------------------------------------------------------------- the rule ----


def _canonical(name: str) -> str:
    lowered = name.strip().lower()
    return _ALIASES.get(lowered, lowered)


def _replace_aliases(expression: str) -> str:
    """Writes known long names as ids, so ``MIT License AND ISC License (ISCL)`` can be parsed."""
    for alias in sorted(_ALIASES, key=len, reverse=True):
        expression = re.sub(re.escape(alias), _ALIASES[alias], expression, flags=re.IGNORECASE)
    return expression


def expression_allowed(expression: str) -> bool:
    """Whether the licence expression lets us ship the package.

    ``A OR B`` needs one good side, ``A AND B`` needs both, ``A WITH exception``
    is as good as ``A``. Anything not understood is refused.
    """
    text = expression.strip()
    if not text:
        return False
    text = _replace_aliases(text)
    tokens: list[str] = []
    position = 0
    for match in _TOKEN.finditer(text):
        before = text[position : match.start()].strip()
        if before:
            tokens.append(before)
        tokens.append(match.group().strip())
        position = match.end()
    tail = text[position:].strip()
    if tail:
        tokens.append(tail)
    try:
        parser = _Parser(tokens)
        result = parser.parse_or()
        return result and parser.finished()
    except _Bad:
        return False


class _Bad(Exception):
    """The expression could not be understood."""


class _Parser:
    def __init__(self, tokens: list[str]) -> None:
        self.tokens = tokens
        self.index = 0

    def finished(self) -> bool:
        return self.index == len(self.tokens)

    def _peek(self) -> str | None:
        return self.tokens[self.index] if self.index < len(self.tokens) else None

    def parse_or(self) -> bool:
        value = self.parse_and()
        while self._peek() == "OR":
            self.index += 1
            right = self.parse_and()
            value = value or right
        return value

    def parse_and(self) -> bool:
        value = self.parse_atom()
        while self._peek() == "AND":
            self.index += 1
            right = self.parse_atom()
            value = value and right
        return value

    def parse_atom(self) -> bool:
        piece = self._peek()
        if piece is None or piece in {"AND", "OR", "WITH", ")"}:
            raise _Bad
        self.index += 1
        if piece == "(":
            value = self.parse_or()
            if self._peek() != ")":
                raise _Bad
            self.index += 1
            return value
        allowed = _canonical(piece) in _ALLOWED
        if self._peek() == "WITH":
            self.index += 1
            exception = self._peek()
            if exception is None or exception in {"AND", "OR", "WITH", "(", ")"}:
                raise _Bad
            self.index += 1
        return allowed


# -------------------------------------------------------- what a package says ----


def licence_of(dist: _Dist) -> str:
    """The licence the package states about itself, or ``UNKNOWN``.

    In order of trust: the licence expression, then the licence classifiers
    (several mean all of them), then a short ``License`` field.
    """
    metadata = dist.metadata
    expression = _get(metadata, "License-Expression")
    if expression and expression.strip():
        return expression.strip()
    classifiers = [
        c.split("::")[-1].strip()
        for c in _get_all(metadata, "Classifier")
        if c.startswith("License ::")
    ]
    classifiers = [c for c in classifiers if c]
    if classifiers:
        return " AND ".join(classifiers)
    field = (_get(metadata, "License") or "").strip()
    if field and "\n" not in field and len(field) <= _MAX_FIELD_CHARS:
        return field
    return "UNKNOWN"


def _get(metadata: object, key: str) -> str | None:
    getter = getattr(metadata, "get", None)
    value = getter(key) if callable(getter) else None
    return value if isinstance(value, str) else None


def _get_all(metadata: object, key: str) -> list[str]:
    getter = getattr(metadata, "get_all", None)
    values = getter(key) if callable(getter) else None
    return [v for v in values if isinstance(v, str)] if isinstance(values, list) else []


# --------------------------------------------------------- what gets shipped ----


def _normalise(name: str) -> str:
    """How pip compares package names: case-insensitive, runs of ``-_.`` equal."""
    return re.sub(r"[-_.]+", "-", name).lower()


def runtime_closure(roots: Iterable[str], installed: Mapping[str, _Dist]) -> list[_Dist]:
    """The packages that ship: the roots and what they need, not extras or other platforms.

    Raises ``LookupError`` naming a needed package that is not installed.
    """
    found: dict[str, _Dist] = {}
    pending = [_normalise(r) for r in roots]
    while pending:
        name = pending.pop()
        if name in found:
            continue
        dist = installed.get(name)
        if dist is None:
            raise LookupError(f"{name} is needed but not installed")
        found[name] = dist
        for line in dist.requires or []:
            try:
                requirement = Requirement(line)
            except InvalidRequirement:
                continue
            if requirement.marker is not None and not requirement.marker.evaluate({"extra": ""}):
                continue
            pending.append(_normalise(requirement.name))
    return list(found.values())


def check(
    roots: Iterable[str],
    installed: Mapping[str, _Dist],
    exceptions: list[LicenceException],
) -> list[Problem]:
    """Every shipped package whose licence is not allowed, and every stale exception."""
    shipped = runtime_closure(roots, installed)
    allowed_by = {(_normalise(e.package), e.licence) for e in exceptions}
    problems: list[Problem] = []
    shipped_names: set[str] = set()
    for dist in shipped:
        name = str(_get(dist.metadata, "Name") or "")
        shipped_names.add(_normalise(name))
        licence = licence_of(dist)
        if expression_allowed(licence) or (_normalise(name), licence) in allowed_by:
            continue
        problems.append(
            Problem(package=name, version=dist.version, licence=licence, kind="refused")
        )
    for exception in exceptions:
        if _normalise(exception.package) not in shipped_names:
            problems.append(
                Problem(
                    package=exception.package,
                    version="",
                    licence=exception.licence,
                    kind="stale exception",
                )
            )
    return problems


# ------------------------------------------------------------------- files ----


def load_exceptions(path: Path) -> list[LicenceException]:
    """The exceptions file; a missing file means none."""
    if not path.exists():
        return []
    try:
        data = tomllib.loads(path.read_text(encoding="utf-8"))
    except (tomllib.TOMLDecodeError, OSError):
        raise ValueError("licence exceptions: the file cannot be read") from None
    entries = data.get("exception", [])
    if not isinstance(entries, list):
        raise ValueError("licence exceptions: 'exception' must be a list of tables")
    result: list[LicenceException] = []
    for entry in entries:
        values = [
            entry.get(k) if isinstance(entry, dict) else None
            for k in ("package", "licence", "reason")
        ]
        if not all(isinstance(v, str) and v.strip() for v in values):
            raise ValueError(
                "licence exceptions: each entry needs a package, the exact licence "
                "wording and a reason"
            )
        result.append(LicenceException(*(str(v) for v in values)))
    return result


def read_requirement_names(path: Path) -> list[str]:
    """The package names in a requirements file (pins and comments ignored)."""
    names: list[str] = []
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line or line.startswith("-"):
            continue
        names.append(Requirement(line).name)
    return names


def main() -> int:
    from importlib import metadata

    installed = {_normalise(d.metadata["Name"]): d for d in metadata.distributions()}
    problems = check(
        read_requirement_names(REQUIREMENTS_FILE), installed, load_exceptions(EXCEPTIONS_FILE)
    )
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        return 1
    print("every package the worker ships has an allowed licence")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
