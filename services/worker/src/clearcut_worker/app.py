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

from fastapi import Depends, FastAPI, Header, Query, Request, Response
from fastapi.concurrency import run_in_threadpool
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse
from pydantic import BaseModel
from starlette.exceptions import HTTPException as StarletteHTTPException

from . import __version__
from .config import Settings
from .images import DEFAULT_MAX_EDGE, MAX_EDGE_LIMIT, EncodedImage, ImageTooLarge
from .inspect import (
    Encrypted,
    InspectError,
    Inspection,
    NotAPdf,
    TooManyPages,
    TooSlow,
    Unreadable,
)
from .isolation import inspect_isolated, ocr_isolated, render_isolated, text_isolated
from .logs import safe_log_text
from .ocr import OcrEngine
from .pagetext import MAX_PAGES_PER_CALL, PageContent, PageOutOfRange, PageTooLarge
from .render import DEFAULT_DPI, MAX_DPI, MIN_DPI, PageTooBig

log = logging.getLogger("clearcut_worker")

# Looks inside a document: (bytes, page cap, seconds) -> what is in it.
Inspector = Callable[[bytes, int, float], Inspection]

# Reads the text of pages: (bytes, first page, last page or None), then the keywords
# words=, max_pages= and timeout_seconds=.
TextReader = Callable[..., list[PageContent]]

# Draws a page: (bytes, page, dpi=, fmt=, max_edge=, max_pages=, timeout_seconds=) -> the picture.
Renderer = Callable[..., EncodedImage]

# Reads a page by OCR: (bytes, page, engine=, agreement=, max_pages=, timeout_seconds=) -> the page.
OcrReader = Callable[..., PageContent]

# The most of an unknown path that is written to a log line.
_LOGGED_PATH_CHARS = 200


class Health(BaseModel):
    status: Literal["ok"]
    version: str
    capabilities: list[str]


class InspectResponse(BaseModel):
    media_type: Literal["application/pdf", "image/png", "image/jpeg"]
    pages: int
    pages_with_text: int
    has_text_layer: bool


class FontOut(BaseModel):
    kind: Literal["type1", "truetype", "type0", "type3", "other"]
    has_to_unicode: bool
    encoding: Literal["none", "winansi", "macroman", "standard", "identity", "custom", "other"]
    embedded: bool


class WordOut(BaseModel):
    start: int
    end: int
    x: int
    y: int
    w: int
    h: int
    confidence: int | None = None


class PageOut(BaseModel):
    page: int
    source: Literal["text_layer", "none", "ocr"]
    text: str
    width_pt: float
    height_pt: float
    fonts: list[FontOut]
    words: list[WordOut] | None = None
    agreement: int | None = None


class TextResponse(BaseModel):
    pages: list[PageOut]


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


def create_app(
    settings: Settings,
    *,
    inspector: Inspector = inspect_isolated,
    text_reader: TextReader = text_isolated,
    renderer: Renderer = render_isolated,
    ocr_engine: OcrEngine | None = None,
    ocr_reader: OcrReader = ocr_isolated,
) -> FastAPI:
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
        # A caller controls the path and could put a newline in it to forge log
        # lines. A matched route is logged by its fixed template; an unknown path
        # is escaped and cut short.
        route = request.scope.get("route")
        path = (
            route.path
            if route is not None
            else safe_log_text(request.scope["path"])[:_LOGGED_PATH_CHARS]
        )
        log.info(
            "%s %s -> %d (%d bytes, %.0f ms)",
            safe_log_text(request.method),
            path,
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

    capabilities = ["inspect", "text", "render"] + (["ocr"] if ocr_engine is not None else [])

    @app.get("/health")
    async def health() -> Health:
        return Health(status="ok", version=__version__, capabilities=capabilities)

    @app.post("/v1/inspect", dependencies=[Depends(require_token)])
    async def inspect(request: Request) -> InspectResponse:
        data = await _read_body(request, settings.max_request_bytes)
        request.state.body_bytes = len(data)
        try:
            result = await run_in_threadpool(
                inspector, data, settings.max_pages, settings.timeout_seconds
            )
        except InspectError as error:
            raise _refusal(error, settings) from None
        return InspectResponse(
            media_type=result.media_type,  # type: ignore[arg-type]
            pages=result.pages,
            pages_with_text=result.pages_with_text,
            has_text_layer=result.has_text_layer,
        )

    @app.post(
        "/v1/text",
        dependencies=[Depends(require_token)],
        response_model_exclude_none=True,
    )
    async def text(
        request: Request,
        first: Annotated[int, Query(alias="from", ge=1)] = 1,
        to: Annotated[int | None, Query(ge=1)] = None,
        words: bool = False,
    ) -> TextResponse:
        if to is not None and (to < first or to - first + 1 > MAX_PAGES_PER_CALL):
            raise WorkerError(400, {"error": "invalid request"})
        data = await _read_body(request, settings.max_request_bytes)
        request.state.body_bytes = len(data)
        try:
            pages = await run_in_threadpool(
                text_reader,
                data,
                first,
                to,
                words=words,
                max_pages=settings.max_pages,
                timeout_seconds=settings.timeout_seconds,
            )
        except InspectError as error:
            raise _refusal(error, settings) from None
        return TextResponse(pages=[_page_out(p) for p in pages])

    @app.post("/v1/ocr", dependencies=[Depends(require_token)], response_model_exclude_none=True)
    async def ocr(
        request: Request,
        page: Annotated[int, Query(ge=1)] = 1,
        agreement: bool = False,
    ) -> TextResponse:
        if ocr_engine is None:
            raise WorkerError(503, {"error": "OCR is not available"})
        data = await _read_body(request, settings.max_request_bytes)
        request.state.body_bytes = len(data)
        try:
            result = await run_in_threadpool(
                ocr_reader,
                data,
                page,
                engine=ocr_engine,
                agreement=agreement,
                max_pages=settings.max_pages,
                timeout_seconds=settings.timeout_seconds,
            )
        except InspectError as error:
            raise _refusal(error, settings) from None
        return TextResponse(pages=[_page_out(result)])

    @app.post("/v1/render", dependencies=[Depends(require_token)])
    async def render(
        request: Request,
        page: Annotated[int, Query(ge=1)] = 1,
        dpi: Annotated[int, Query(ge=MIN_DPI, le=MAX_DPI)] = DEFAULT_DPI,
        fmt: Annotated[Literal["png", "jpeg"], Query(alias="format")] = "png",
        max_edge: Annotated[int, Query(ge=500, le=MAX_EDGE_LIMIT)] = DEFAULT_MAX_EDGE,
    ) -> Response:
        data = await _read_body(request, settings.max_request_bytes)
        request.state.body_bytes = len(data)
        try:
            image = await run_in_threadpool(
                renderer,
                data,
                page,
                dpi=dpi,
                fmt=fmt,
                max_edge=max_edge,
                max_pages=settings.max_pages,
                timeout_seconds=settings.timeout_seconds,
            )
        except InspectError as error:
            raise _refusal(error, settings) from None
        return Response(
            content=image.data,
            media_type=image.media_type,
            headers={
                "X-Image-Width": str(image.width),
                "X-Image-Height": str(image.height),
                # A page of someone's invoice: nothing should keep a copy.
                "Cache-Control": "no-store",
            },
        )

    return app


def _page_out(page: PageContent) -> PageOut:
    return PageOut(
        page=page.page,
        source=page.source,  # type: ignore[arg-type]
        text=page.text,
        width_pt=page.width_pt,
        height_pt=page.height_pt,
        fonts=[
            FontOut(
                kind=f.kind,  # type: ignore[arg-type]
                has_to_unicode=f.has_to_unicode,
                encoding=f.encoding,  # type: ignore[arg-type]
                embedded=f.embedded,
            )
            for f in page.fonts
        ],
        words=(
            None
            if page.words is None
            else [
                WordOut(
                    start=w.start, end=w.end, x=w.x, y=w.y, w=w.w, h=w.h, confidence=w.confidence
                )
                for w in page.words
            ]
        ),
        agreement=page.agreement,
    )


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
    if isinstance(error, PageOutOfRange):
        return WorkerError(
            422, {"error": "the page range is outside the document", "pages": error.pages}
        )
    if isinstance(error, PageTooLarge):
        return WorkerError(422, {"error": "a page holds more text than the worker will report"})
    if isinstance(error, PageTooBig):
        return WorkerError(422, {"error": "the page is too large to render"})
    if isinstance(error, ImageTooLarge):
        return WorkerError(422, {"error": "the image is too large"})
    return WorkerError(500, {"error": "internal error"})
