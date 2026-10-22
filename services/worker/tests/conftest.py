# Written by Kone Tshivhinda & Claude
"""Shared test helpers: tiny PDFs built by hand, and a client for the app."""

from __future__ import annotations

import io

import pytest
from fastapi.testclient import TestClient
from pypdf import PdfWriter

from clearcut_worker.app import create_app
from clearcut_worker.config import Settings

TOKEN = "t" * 40
AUTH = {"Authorization": f"Bearer {TOKEN}"}


def make_pdf(pages: list[str | None]) -> bytes:
    """A small valid PDF with one page per entry: the text on it, or None for a blank page."""
    objects: list[bytes] = []
    n = len(pages)
    kids = " ".join(f"{4 + 2 * i} 0 R" for i in range(n))
    objects.append(b"<< /Type /Catalog /Pages 2 0 R >>")
    objects.append(f"<< /Type /Pages /Kids [{kids}] /Count {n} >>".encode())
    objects.append(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    for i, text in enumerate(pages):
        content_id = 5 + 2 * i
        objects.append(
            (
                f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] "
                f"/Resources << /Font << /F1 3 0 R >> >> /Contents {content_id} 0 R >>"
            ).encode()
        )
        stream = b"" if text is None else f"BT /F1 12 Tf 20 100 Td ({text}) Tj ET".encode()
        objects.append(b"<< /Length %d >>\nstream\n%s\nendstream" % (len(stream), stream))

    out = io.BytesIO()
    out.write(b"%PDF-1.4\n")
    offsets = []
    for number, body in enumerate(objects, start=1):
        offsets.append(out.tell())
        out.write(b"%d 0 obj\n%s\nendobj\n" % (number, body))
    xref = out.tell()
    out.write(b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1))
    for offset in offsets:
        out.write(b"%010d 00000 n \n" % offset)
    out.write(
        b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objects) + 1, xref)
    )
    return out.getvalue()


def make_encrypted_pdf() -> bytes:
    writer = PdfWriter()
    writer.add_blank_page(72, 72)
    writer.encrypt("a-password", algorithm="RC4-128")
    out = io.BytesIO()
    writer.write(out)
    return out.getvalue()


def settings(
    *,
    token: str = TOKEN,
    max_request_bytes: int = 1024 * 1024,
    max_pages: int = 50,
    timeout_seconds: float = 30.0,
) -> Settings:
    return Settings(
        token=token,
        max_request_bytes=max_request_bytes,
        max_pages=max_pages,
        timeout_seconds=timeout_seconds,
    )


@pytest.fixture
def client() -> TestClient:
    return TestClient(create_app(settings()))
