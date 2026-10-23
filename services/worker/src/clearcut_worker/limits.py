# Written by Kone Tshivhinda & Claude
"""What a worker child process is allowed to take, and to see (ADR-0016).

The child parses a hostile file with native code (a PDF engine, later an OCR
program). Time is limited by the parent, which kills the child at a deadline.
This module limits the rest, from inside the child, before it touches the file:

- **memory** (the address space), so a file that tries to make the parser eat
  all the memory gets an ordinary out-of-memory error in the child instead of
  taking down the machine;
- **processor time**, a backstop for a loop that ignores the clock;
- **open files**, and **the size of any file it writes**, so scratch space
  cannot be filled;
- **core dumps**, switched off: a crash must not write the document's memory to disk.

The limits are set with ``resource``, which Windows does not have; there the
container (P1-07) is what limits the child, and nothing here pretends otherwise.
A limit that is supposed to be set and cannot be is an **error**, never a shrug:
the child then does not touch the document.

``scrub_environment`` removes everything from the child's environment except a
short allowlist, so a compromised parser cannot read the worker's bearer token
or any other secret that happens to be in the process environment, and the
programs it starts inherit nothing either.
"""

from __future__ import annotations

import importlib
from collections.abc import MutableMapping
from dataclasses import dataclass
from typing import Any


class LimitError(Exception):
    """A limit that must be set could not be. Names the limit, never a value."""


@dataclass(frozen=True)
class ChildLimits:
    address_space_bytes: int = 2 * 1024**3
    cpu_seconds: int = 60
    open_files: int = 64
    file_size_bytes: int = 256 * 1024**2

    def __post_init__(self) -> None:
        for name in ("address_space_bytes", "cpu_seconds", "open_files", "file_size_bytes"):
            if getattr(self, name) <= 0:
                raise ValueError(f"the {name} limit must be greater than zero")


def _load_resource() -> Any:
    try:
        return importlib.import_module("resource")
    except ImportError:
        return None  # Windows


_RESOURCE = _load_resource()


def apply_limits(limits: ChildLimits, *, resource_module: Any = _RESOURCE) -> list[str]:
    """Sets the limits on this process and returns the names of those that were set.

    A limit is never raised above what the system already allows. With no
    ``resource`` module (Windows) nothing is set and the answer is empty.
    """
    if resource_module is None:
        return []
    wanted = [
        ("memory", "RLIMIT_AS", limits.address_space_bytes),
        ("cpu", "RLIMIT_CPU", limits.cpu_seconds),
        ("open_files", "RLIMIT_NOFILE", limits.open_files),
        ("file_size", "RLIMIT_FSIZE", limits.file_size_bytes),
        ("core", "RLIMIT_CORE", 0),
    ]
    applied: list[str] = []
    for name, constant, value in wanted:
        which = getattr(resource_module, constant, None)
        if which is None:
            continue
        try:
            _soft, hard = resource_module.getrlimit(which)
            target = value if hard == resource_module.RLIM_INFINITY else min(value, hard)
            resource_module.setrlimit(which, (target, target))
        except ValueError:
            # This platform does not support that limit (macOS has no address-space limit).
            continue
        except OSError:
            raise LimitError(f"the {name} limit could not be set") from None
        applied.append(name)
    return applied


# What the child may keep in its environment. Everything else is removed.
_KEPT = frozenset(
    {
        "PATH",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "WINDIR",
        "TEMP",
        "TMP",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "TESSDATA_PREFIX",
        "PYTHONUTF8",
        "PYTHONIOENCODING",
    }
)


def scrub_environment(env: MutableMapping[str, str]) -> list[str]:
    """Removes every variable not on the allowlist and returns the names removed."""
    removed = [name for name in env if name.upper() not in _KEPT]
    for name in removed:
        del env[name]
    return removed
