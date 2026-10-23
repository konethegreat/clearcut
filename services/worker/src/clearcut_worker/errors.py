# Written by Kone Tshivhinda & Claude
"""The named ways a document is refused. None of their messages ever comes from the file."""

from __future__ import annotations


class InspectError(Exception):
    """Something stops the document from being read."""


class NotAPdf(InspectError):
    """The bytes are not a kind of file the worker reads (a PDF, a PNG or a JPEG)."""


class Unreadable(InspectError):
    """The file claims to be a kind the worker reads but cannot be read (damaged, or not valid)."""


class Encrypted(InspectError):
    """The PDF is password-protected. We never try passwords."""


class TooManyPages(InspectError):
    def __init__(self, max_pages: int) -> None:
        super().__init__("too many pages")
        self.max_pages = max_pages


class TooSlow(InspectError):
    """The deadline passed before reading finished."""
