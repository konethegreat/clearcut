# Written by Kone Tshivhinda & Claude
"""Test PDFs built by hand, with exactly the fonts, boxes and rotation a test needs.

The objects are given by name and refer to each other with ``@name@``; the numbers
and the cross-reference table are worked out here. Nothing in this module is a
real document: every text is invented.
"""

from __future__ import annotations

import io
import re
from dataclasses import dataclass, field

BS = chr(92)  # a backslash, which PDF strings use to escape
HELVETICA = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"


@dataclass(frozen=True)
class PageSpec:
    """One page: its drawing instructions and the boxes around it."""

    content: str
    media_box: str = "[0 0 200 200]"
    crop_box: str | None = None
    rotate: int = 0
    resources: str = "/Font << /F1 @helvetica@ >>"


def text_at(x: float, y: float, text: str, *, size: int = 12, font: str = "F1") -> str:
    """Drawing instructions that put ``text`` with its baseline origin at (x, y)."""
    safe = text.replace(BS, BS + BS).replace("(", BS + "(").replace(")", BS + ")")
    return f"BT /{font} {size} Tf {x} {y} Td ({safe}) Tj ET\n"


@dataclass
class PdfBuilder:
    pages: list[PageSpec] = field(default_factory=list)
    objects: dict[str, str | bytes] = field(default_factory=dict)

    def build(self) -> bytes:
        named: dict[str, str | bytes] = {"helvetica": HELVETICA, **self.objects}
        # Numbers: 1 catalog, 2 page tree, then the named objects, then two per page.
        numbers = {name: 3 + index for index, name in enumerate(named)}
        first_page = 3 + len(named)

        def resolve(body: str | bytes) -> bytes:
            raw = body.encode("latin-1") if isinstance(body, str) else body
            return re.sub(
                rb"@([A-Za-z0-9_]+)@",
                lambda m: b"%d 0 R" % numbers[m.group(1).decode()],
                raw,
            )

        kids = " ".join(f"{first_page + 2 * i} 0 R" for i in range(len(self.pages)))
        bodies: list[bytes] = [
            b"<< /Type /Catalog /Pages 2 0 R >>",
            f"<< /Type /Pages /Kids [{kids}] /Count {len(self.pages)} >>".encode(),
        ]
        bodies += [resolve(body) for body in named.values()]
        for index, page in enumerate(self.pages):
            content_id = first_page + 2 * index + 1
            entries = [
                "/Type /Page",
                "/Parent 2 0 R",
                f"/MediaBox {page.media_box}",
                f"/Resources << {page.resources} >>",
                f"/Contents {content_id} 0 R",
            ]
            if page.crop_box is not None:
                entries.append(f"/CropBox {page.crop_box}")
            if page.rotate:
                entries.append(f"/Rotate {page.rotate}")
            bodies.append(resolve("<< " + " ".join(entries) + " >>"))
            stream = page.content.encode("latin-1")
            bodies.append(b"<< /Length %d >>\nstream\n%s\nendstream" % (len(stream), stream))

        out = io.BytesIO()
        out.write(b"%PDF-1.4\n")
        offsets = []
        for number, body in enumerate(bodies, start=1):
            offsets.append(out.tell())
            out.write(b"%d 0 obj\n%s\nendobj\n" % (number, body))
        xref = out.tell()
        out.write(b"xref\n0 %d\n0000000000 65535 f \n" % (len(bodies) + 1))
        for offset in offsets:
            out.write(b"%010d 00000 n \n" % offset)
        out.write(
            b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n"
            % (len(bodies) + 1, xref)
        )
        return out.getvalue()


def one_page(content: str, **options: object) -> bytes:
    """A one-page PDF drawing ``content`` in Helvetica as font F1."""
    return PdfBuilder(pages=[PageSpec(content=content, **options)]).build()  # type: ignore[arg-type]


def pages_of_text(*texts: str | None) -> bytes:
    """A PDF with one page per text (None for a blank page), each line near the left edge."""
    specs = [PageSpec(content="" if text is None else text_at(20, 100, text)) for text in texts]
    return PdfBuilder(pages=specs).build()
