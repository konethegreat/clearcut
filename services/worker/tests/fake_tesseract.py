# Written by Kone Tshivhinda & Claude
"""A stand-in for the Tesseract program, so the OCR tests can run on any machine.

It is copied next to a ``scenario.json`` that says what it should do, then started the
way the worker starts the real program. It writes what it was asked (arguments, working
directory, environment) to ``record.json`` so a test can check the worker called it with
a fixed argument list, in the scratch directory, with nothing secret in its environment.
"""

from __future__ import annotations

import json
import os
import struct
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent


def main() -> int:
    scenario = json.loads((HERE / "scenario.json").read_text(encoding="utf-8"))
    record = {
        "argv": sys.argv[1:],
        "cwd": os.getcwd(),
        "env": dict(os.environ),
        "image_exists": bool(sys.argv[1:]) and Path(sys.argv[1]).exists(),
        "image_head": (
            Path(sys.argv[1]).read_bytes()[:8].hex()
            if len(sys.argv) > 1 and Path(sys.argv[1]).exists()
            else ""
        ),
    }
    if record["image_head"] == "89504e470d0a1a0a":
        header = Path(sys.argv[1]).read_bytes()[16:24]
        record["image_wh"] = list(struct.unpack(">II", header))
    with (HERE / "record.json").open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(record) + "\n")

    if "--list-langs" in sys.argv:
        if scenario.get("hang_listing"):
            time.sleep(120)
        print(f'List of available languages in "/somewhere" ({len(scenario["langs"])}):')
        for language in scenario["langs"]:
            print(language)
        return int(scenario.get("exit", 0))

    if scenario.get("hang"):
        time.sleep(120)
    print(scenario.get("stderr", ""), file=sys.stderr)
    if scenario.get("no_output"):
        return int(scenario.get("exit", 0))
    base = sys.argv[2]
    body = scenario.get("tsv", "")
    if scenario.get("big"):
        body += "x" * scenario["big"]
    Path(base + ".tsv").write_text(body, encoding="utf-8", newline="")
    return int(scenario.get("exit", 0))


if __name__ == "__main__":
    raise SystemExit(main())
