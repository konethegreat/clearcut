# Written by Kone Tshivhinda & Claude
"""The HTTP application.

Rules, each with a test:

- **Authenticate first.** Everything except ``/health`` needs
  ``Authorization: Bearer <token>``. Every failure is the same ``401``, and the
  check happens before the body is read, so an anonymous caller learns nothing
  about the limits or the parser.
- **Never trust the body.** The file type comes from the bytes, not from what the
  caller says it is. The size cap is enforced as the bytes arrive, whatever
  length the caller claimed.
- **Never repeat or log content.** Errors say which rule failed; logs carry
  method, path, status, size and time, never the file or the token.
- **Errors are JSON** with one key, ``error``, like the Rust API's.
- **The API description is not served** (no ``/docs``, ``/openapi.json``): it is
  generated for the contract test only.
"""

from __future__ import annotations

import hmac
import logging
import time
from collections.abc import Awaitable, Callable
from typing import Annotated, Literal

from fastapi import Depends, FastAPI, Header, Request, Response
from fastapi.concurrency import run_in_threadpool
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse
from pydantic import BaseModel
from starlette.exceptions import HTTPException as StarletteHTTPException

from . import __version__
from .config import Settings
from .inspect import (
    Encrypted,
    InspectError,
    NotAPdf,
    TooManyPages,
    TooSlow,
    Unreadable,
    inspect_pdf,
)

log = logging.getLogger("clearcut_worker")


class Health(BaseModel):
    status: Literal["ok"]
    version: str
    capabilities: list[str]


class InspectResponse(BaseModel):
    media_type: Literal["application/pdf"]
    pages: int
    pages_with_text: int
    has_text_layer: bool


class WorkerError(Exception):
    """An answer that is not a success: a status, a JSON body, maybe headers."""

    def __init__(
        self, status: int, body: dict[str, object], headers: dict[str, str] | None = None
    ) -> None:
        super().__init__(status)
        self.status = status
        self.body = body
        self.headers = headers or {}


_UNAUTHORIZED = WorkerError(401, {"error": "authentication failed"}, {"WWW-Authenticate": "Bearer"})


def _bearer_token(header: str | None) -> str | None:
    """Whatever follows the scheme name ``Bearer`` (case-insensitive), or None.

    Nothing else is checked here on purpose: a missing, empty, padded or
    multi-word token can never equal the real one (which has no whitespace), so
    the constant-time comparison in ``require_token`` rejects it.
    """
    if header is None:
        return None
    scheme, _, token = header.partition(" ")
    return token if scheme.lower() == "bearer" else None


def create_app(settings: Settings) -> FastAPI:
    app = FastAPI(
        title="ClearCut document worker",
        version=__version__,
        docs_url=None,
        redoc_url=None,
        openapi_url=None,
    )

    def require_token(authorization: Annotated[str | None, Header()] = None) -> None:
        presented = _bearer_token(authorization)
        # Compare as bytes, in constant time, so timing reveals nothing about the token.
        if presented is None or not hmac.compare_digest(
            presented.encode(), settings.token.encode()
        ):
            raise _UNAUTHORIZED

    @app.middleware("http")
    async def log_request(
        request: Request, call_next: Callable[[Request], Awaitable[Response]]
    ) -> Response:
        started = time.monotonic()
        request.state.body_bytes = 0
        response = await call_next(request)
        log.info(
            "%s %s -> %d (%d bytes, %.0f ms)",
            request.method,
            request.url.path,
            response.status_code,
            request.state.body_bytes,
            (time.monotonic() - started) * 1000,
        )
        return response

    @app.exception_handler(WorkerError)
    async def worker_error(_: Request, error: WorkerError) -> JSONResponse:
        return JSONResponse(error.body, status_code=error.status, headers=error.headers)

    @app.exception_handler(StarletteHTTPException)
    async def http_error(_: Request, error: StarletteHTTPException) -> JSONResponse:
        text = {404: "not found", 405: "method not allowed"}.get(
            error.status_code, "request failed"
        )
        return JSONResponse({"error": text}, status_code=error.status_code, headers=error.headers)

    @app.exception_handler(RequestValidationError)
    async def invalid_request(_: Request, __: RequestValidationError) -> JSONResponse:
        # The details could echo what the caller sent; say only that it was wrong.
        return JSONResponse({"error": "invalid request"}, status_code=400)

    @app.exception_handler(Exception)
    async def unexpected(_: Request, error: Exception) -> JSONResponse:
        log.error("unexpected %s", type(error).__name__)
        return JSONResponse({"error": "internal error"}, status_code=500)

    @app.get("/health")
    async def health() -> Health:
        return Health(status="ok", version=__version__, capabilities=["inspect"])

    @app.post("/v1/inspect", dependencies=[Depends(require_token)])
    async def inspect(request: Request) -> InspectResponse:
        data = await _read_body(request, settings.max_request_bytes)
        request.state.body_bytes = len(data)
        deadline = time.monotonic() + settings.timeout_seconds
        try:
            result = await run_in_threadpool(inspect_pdf, data, settings.max_pages, deadline)
        except InspectError as error:
            raise _refusal(error, settings) from None
        return InspectResponse(
            media_type="application/pdf",
            pages=result.pages,
            pages_with_text=result.pages_with_text,
            has_text_layer=result.has_text_layer,
        )

    return app


async def _read_body(request: Request, limit: int) -> bytes:
    """The whole body, refusing it as soon as it is known to exceed ``limit``."""
    too_large = WorkerError(413, {"error": "file too large", "max_bytes": limit})
    declared = request.headers.get("content-length")
    if declared is not None and declared.isdigit() and int(declared) > limit:
        raise too_large
    chunks: list[bytes] = []
    total = 0
    async for chunk in request.stream():
        total += len(chunk)
        if total > limit:
            raise too_large
        chunks.append(chunk)
    if total == 0:
        raise WorkerError(400, {"error": "the file is empty"})
    return b"".join(chunks)


def _refusal(error: InspectError, settings: Settings) -> WorkerError:
    if isinstance(error, NotAPdf):
        return WorkerError(415, {"error": "unsupported file type"})
    if isinstance(error, Encrypted):
        return WorkerError(422, {"error": "the document is encrypted"})
    if isinstance(error, Unreadable):
        return WorkerError(422, {"error": "the document could not be read"})
    if isinstance(error, TooManyPages):
        return WorkerError(422, {"error": "too many pages", "max_pages": settings.max_pages})
    if isinstance(error, TooSlow):
        return WorkerError(504, {"error": "the document took too long to inspect"})
    return WorkerError(500, {"error": "internal error"})
