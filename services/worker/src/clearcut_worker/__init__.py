# Written by Kone Tshivhinda & Claude
"""The ClearCut document worker (ADR-0009).

It reads untrusted documents and tells the Rust service what is in them (page
counts, text layers; later OCR and page images). It holds no API key, makes no
outbound network calls, touches no database and decides nothing about what an
invoice means: validation and every business rule live in ``clearcut-core``.
"""

__version__ = "0.1.0"
