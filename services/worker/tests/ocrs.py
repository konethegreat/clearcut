# Written by Kone Tshivhinda & Claude
"""Helpers for the OCR tests: TSV text as Tesseract writes it, and a fake or real engine."""

from __future__ import annotations

import json
import shutil
import sys
from pathlib import Path
from typing import Any

from clearcut_worker.ocr import OcrEngine, find_engine

HEADER = (
    "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext"
)


def row(
    text: str,
    left: int,
    top: int,
    width: int,
    height: int,
    conf: float = 96.5,
    *,
    block: int = 1,
    par: int = 1,
    line: int = 1,
    word: int = 1,
    level: int = 5,
) -> str:
    numbers = [level, 1, block, par, line, word, left, top, width, height, conf]
    return "\t".join([*(str(n) for n in numbers), text])


def tsv(*rows: str, newline: str = "\n") -> str:
    """A TSV with its header, the page-level rows Tesseract also writes, and the given rows."""
    structure = [
        "1\t1\t0\t0\t0\t0\t0\t0\t1000\t800\t-1\t",
        "2\t1\t1\t0\t0\t0\t10\t10\t900\t100\t-1\t",
        "3\t1\t1\t1\t0\t0\t10\t10\t900\t100\t-1\t",
        "4\t1\t1\t1\t1\t0\t10\t10\t900\t50\t-1\t",
    ]
    return newline.join([HEADER, *structure, *rows]) + newline


def fake_engine(directory: Path, **scenario: Any) -> OcrEngine:
    """A copy of the fake program in ``directory`` with its scenario beside it."""
    source = Path(__file__).resolve().parent / "fake_tesseract.py"
    directory.mkdir(parents=True, exist_ok=True)
    shutil.copy(source, directory / "fake_tesseract.py")
    scenario.setdefault("langs", ["afr", "eng", "osd"])
    (directory / "scenario.json").write_text(json.dumps(scenario), encoding="utf-8")
    return OcrEngine(command=(sys.executable, str(directory / "fake_tesseract.py")))


def records(directory: Path) -> list[dict[str, Any]]:
    path = directory / "record.json"
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]


def real_engine() -> OcrEngine | None:
    """The real Tesseract with English and Afrikaans, or None where it is not installed."""
    return find_engine(None, None)
