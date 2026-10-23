# Written by Kone Tshivhinda & Claude
"""The worker over HTTP: who may ask, what it answers, and what it refuses."""

from __future__ import annotations

import logging
from collections.abc import Iterator

import pytest
from fastapi.testclient import TestClient

from clearcut_worker import __version__
from clearcut_worker.app import create_app
from clearcut_worker.inspect import (
    Encrypted,
    InspectError,
    Inspection,
    NotAPdf,
    TooManyPages,
    TooSlow,
    Unreadable,
)

from .conftest import AUTH, TOKEN, make_encrypted_pdf, make_pdf, settings
from .pictures import claims_to_be, jpeg, png

INSPECT = "/v1/inspect"
UNAUTHORIZED = {"error": "authentication failed"}


# ---------- health ----------


def test_health_is_public_and_says_what_the_worker_can_do(client: TestClient) -> None:
    response = client.get("/health")
    assert response.status_code == 200
    assert response.json() == {
        "status": "ok",
        "version": __version__,
        "capabilities": ["inspect", "text", "render"],
    }


def test_health_needs_no_key_and_ignores_a_wrong_one(client: TestClient) -> None:
    assert client.get("/health", headers={"Authorization": "Bearer nope"}).status_code == 200


# ---------- authentication ----------


@pytest.mark.parametrize(
    "header",
    [
        None,
        "",
        "Bearer",
        "Bearer ",
        "Bearer wrong",
        "Bearer " + "t" * 39,
        "Bearer " + "t" * 41,
        "Basic " + "t" * 40,
        "Token " + "t" * 40,
        "t" * 40,
        "Bearer  " + "t" * 40,
        "Bearer " + "t" * 40 + " extra",
    ],
)
def test_every_kind_of_bad_credential_gets_the_same_401(
    client: TestClient, header: str | None
) -> None:
    headers = {} if header is None else {"Authorization": header}
    response = client.post(INSPECT, content=make_pdf(["x"]), headers=headers)
    assert response.status_code == 401
    assert response.json() == UNAUTHORIZED
    assert response.headers["www-authenticate"] == "Bearer"


def test_the_scheme_name_is_case_insensitive(client: TestClient) -> None:
    for scheme in ["bearer", "BEARER", "Bearer"]:
        response = client.post(
            INSPECT, content=make_pdf(["x"]), headers={"Authorization": f"{scheme} {TOKEN}"}
        )
        assert response.status_code == 200, scheme


def test_authentication_comes_before_the_body_is_read() -> None:
    # An anonymous caller claiming a huge body gets a 401, not a 413: the worker
    # does not even look at how big it is.
    app = TestClient(create_app(settings(max_request_bytes=10)))
    response = app.post(INSPECT, content=b"x" * 1000)
    assert response.status_code == 401
    response = app.post(INSPECT, content=make_encrypted_pdf())
    assert response.status_code == 401


def test_the_other_methods_and_unknown_paths_answer_in_json_too(client: TestClient) -> None:
    wrong_method = [
        client.get(INSPECT, headers=AUTH),
        client.put(INSPECT, headers=AUTH, content=b"x"),
    ]
    for response in wrong_method:
        assert response.status_code == 405
        assert response.json() == {"error": "method not allowed"}
    for response in [client.get("/nope"), client.post("/v1/nothing", headers=AUTH)]:
        assert response.status_code == 404
        assert response.json() == {"error": "not found"}


def test_the_api_description_is_not_served_to_the_network(client: TestClient) -> None:
    for path in ["/docs", "/redoc", "/openapi.json"]:
        assert client.get(path).status_code == 404, path
        assert client.get(path, headers=AUTH).status_code == 404, path


def test_the_api_description_can_still_be_generated_for_the_contract_test() -> None:
    spec = create_app(settings()).openapi()
    assert set(spec["paths"]) == {
        "/health",
        "/v1/inspect",
        "/v1/text",
        "/v1/render",
        "/v1/ocr",
    }


# ---------- inspect ----------


def test_a_pdf_is_inspected(client: TestClient) -> None:
    response = client.post(INSPECT, content=make_pdf(["Invoice", None, "Total"]), headers=AUTH)
    assert response.status_code == 200
    assert response.json() == {
        "media_type": "application/pdf",
        "pages": 3,
        "pages_with_text": 2,
        "has_text_layer": True,
    }


def test_a_scan_without_text_says_so(client: TestClient) -> None:
    body = client.post(INSPECT, content=make_pdf([None, None]), headers=AUTH).json()
    assert (body["pages"], body["pages_with_text"], body["has_text_layer"]) == (2, 0, False)


def test_the_type_comes_from_the_bytes_not_from_what_the_caller_claims(client: TestClient) -> None:
    pdf = make_pdf(["x"])
    for claimed in ["text/html", "image/png", "application/octet-stream", "application/pdf"]:
        response = client.post(INSPECT, content=pdf, headers={**AUTH, "Content-Type": claimed})
        assert response.status_code == 200, claimed
    response = client.post(
        INSPECT,
        content=b"<html>not a pdf</html>",
        headers={**AUTH, "Content-Type": "application/pdf"},
    )
    assert response.status_code == 415
    assert response.json() == {"error": "unsupported file type"}


def test_other_file_kinds_are_not_supported(client: TestClient) -> None:
    for blob in [b"II*\x00" + b"0" * 50, b"GIF89a" + b"0" * 50, b"RIFF0000WEBP" + b"0" * 50]:
        response = client.post(INSPECT, content=blob, headers=AUTH)
        assert response.status_code == 415


def test_a_png_or_jpeg_that_is_only_a_signature_is_unreadable_not_unsupported(
    client: TestClient,
) -> None:
    for blob in [b"\x89PNG\r\n\x1a\n" + b"0" * 50, b"\xff\xd8\xff\xe0" + b"0" * 50]:
        response = client.post(INSPECT, content=blob, headers=AUTH)
        assert response.status_code == 422
        assert response.json() == {"error": "the document could not be read"}


def test_an_empty_body_is_a_400(client: TestClient) -> None:
    response = client.post(INSPECT, content=b"", headers=AUTH)
    assert response.status_code == 400
    assert response.json() == {"error": "the file is empty"}


# ---------- limits ----------


def test_a_file_over_the_size_cap_is_refused_from_its_declared_length() -> None:
    app = TestClient(create_app(settings(max_request_bytes=100)))
    response = app.post(INSPECT, content=b"%PDF-" + b"x" * 200, headers=AUTH)
    assert response.status_code == 413
    assert response.json() == {"error": "file too large", "max_bytes": 100}


def test_a_file_exactly_at_the_cap_is_allowed() -> None:
    pdf = make_pdf(["x"])
    app = TestClient(create_app(settings(max_request_bytes=len(pdf))))
    assert app.post(INSPECT, content=pdf, headers=AUTH).status_code == 200
    app = TestClient(create_app(settings(max_request_bytes=len(pdf) - 1)))
    assert app.post(INSPECT, content=pdf, headers=AUTH).status_code == 413


def test_a_body_that_lies_about_its_length_is_still_cut_off() -> None:
    # No Content-Length at all (chunked): the cap is enforced as the bytes arrive.
    app = TestClient(create_app(settings(max_request_bytes=100)))

    def chunks() -> Iterator[bytes]:
        yield b"%PDF-"
        for _ in range(50):
            yield b"x" * 100

    response = app.post(INSPECT, content=chunks(), headers=AUTH)
    assert response.status_code == 413
    assert response.json()["max_bytes"] == 100


def test_too_many_pages_is_a_422_that_names_the_cap() -> None:
    app = TestClient(create_app(settings(max_pages=3)))
    assert app.post(INSPECT, content=make_pdf(["a"] * 3), headers=AUTH).status_code == 200
    response = app.post(INSPECT, content=make_pdf(["a"] * 4), headers=AUTH)
    assert response.status_code == 422
    assert response.json() == {"error": "too many pages", "max_pages": 3}


def test_a_document_that_takes_too_long_is_a_504() -> None:
    app = TestClient(create_app(settings(timeout_seconds=0.0)))
    response = app.post(INSPECT, content=make_pdf(["a", "b"]), headers=AUTH)
    assert response.status_code == 504
    assert response.json() == {"error": "the document took too long to inspect"}


# ---------- bad documents ----------


def test_an_encrypted_pdf_is_a_422(client: TestClient) -> None:
    response = client.post(INSPECT, content=make_encrypted_pdf(), headers=AUTH)
    assert response.status_code == 422
    assert response.json() == {"error": "the document is encrypted"}


def test_a_damaged_pdf_is_a_422_that_reveals_nothing_about_the_parser(client: TestClient) -> None:
    response = client.post(
        INSPECT, content=b"%PDF-1.4\nSECRET-PAYLOAD 1234567890 garbage", headers=AUTH
    )
    assert response.status_code == 422
    assert response.json() == {"error": "the document could not be read"}
    assert "SECRET" not in response.text


# ---------- what it never says or logs ----------


def test_no_error_ever_echoes_the_request_content(client: TestClient) -> None:
    secret = b"%PDF-1.4\nSECRET-PAYLOAD bank 1234567890"
    for response in [
        client.post(INSPECT, content=secret, headers=AUTH),
        client.post(INSPECT, content=secret),
        client.post(INSPECT, content=b"SECRET-PAYLOAD", headers=AUTH),
    ]:
        assert "SECRET" not in response.text
        assert "1234567890" not in response.text


def test_logs_carry_sizes_and_outcomes_but_never_content_or_the_token(
    client: TestClient, caplog: pytest.LogCaptureFixture
) -> None:
    caplog.set_level(logging.DEBUG)
    body = make_pdf(["SECRET-INVOICE-TEXT"])
    client.post(INSPECT, content=body, headers=AUTH)
    client.post(
        INSPECT, content=b"SECRET-PAYLOAD", headers={"Authorization": "Bearer SECRET-WRONG-TOKEN"}
    )
    # Only the worker's own lines: the test client logs its requests too.
    lines = [r.getMessage() for r in caplog.records if r.name == "clearcut_worker"]
    logged = "\n".join(lines)
    assert "SECRET" not in logged
    assert TOKEN not in logged
    assert len(lines) == 2
    assert lines[0].startswith(f"POST /v1/inspect -> 200 ({len(body)} bytes, ")
    assert lines[1].startswith("POST /v1/inspect -> 401 (0 bytes, ")


# ---------- handlers for things that should not happen ----------


def test_a_request_the_framework_rejects_gets_our_error_shape_and_no_details() -> None:
    app = create_app(settings())

    async def numbered(n: int) -> dict[str, int]:
        return {"n": n}

    app.add_api_route("/test/{n}", numbered)
    response = TestClient(app).get("/test/SECRET-not-a-number")
    assert response.status_code == 400
    assert response.json() == {"error": "invalid request"}
    assert "SECRET" not in response.text


def test_an_unexpected_failure_is_a_500_that_says_nothing_and_logs_only_its_type(
    caplog: pytest.LogCaptureFixture,
) -> None:
    app = create_app(settings())

    async def boom() -> None:
        raise RuntimeError("SECRET-DETAIL 1234567890")

    app.add_api_route("/test/boom", boom)
    caplog.set_level(logging.DEBUG)
    response = TestClient(app, raise_server_exceptions=False).get("/test/boom")
    assert response.status_code == 500
    assert response.json() == {"error": "internal error"}
    logged = "\n".join(r.getMessage() for r in caplog.records if r.name == "clearcut_worker")
    assert "RuntimeError" in logged
    assert "SECRET" not in logged
    assert "SECRET" not in response.text


# ---------- the inspector can be swapped, and is called as promised ----------


def test_the_app_hands_the_inspector_the_bytes_the_page_cap_and_the_timeout() -> None:
    seen: list[tuple[bytes, int, float]] = []

    def fake(data: bytes, max_pages: int, timeout_seconds: float) -> Inspection:
        seen.append((data, max_pages, timeout_seconds))
        return Inspection(pages=7, pages_with_text=4)

    app = TestClient(create_app(settings(max_pages=12, timeout_seconds=3.5), inspector=fake))
    response = app.post(INSPECT, content=b"whatever bytes", headers=AUTH)
    assert response.status_code == 200
    assert response.json() == {
        "media_type": "application/pdf",
        "pages": 7,
        "pages_with_text": 4,
        "has_text_layer": True,
    }
    assert seen == [(b"whatever bytes", 12, 3.5)]


def test_whatever_the_inspector_refuses_with_is_translated() -> None:
    cases: list[tuple[InspectError, int, dict[str, object]]] = [
        (NotAPdf(), 415, {"error": "unsupported file type"}),
        (Encrypted(), 422, {"error": "the document is encrypted"}),
        (Unreadable(), 422, {"error": "the document could not be read"}),
        (TooManyPages(99), 422, {"error": "too many pages", "max_pages": 3}),
        (TooSlow(), 504, {"error": "the document took too long to inspect"}),
    ]
    for error, status, body in cases:

        def refuse(
            data: bytes, max_pages: int, timeout_seconds: float, error: InspectError = error
        ) -> Inspection:
            raise error

        app = TestClient(create_app(settings(max_pages=3), inspector=refuse))
        response = app.post(INSPECT, content=b"x", headers=AUTH)
        assert (response.status_code, response.json()) == (status, body), type(error).__name__


def test_a_route_with_a_path_parameter_is_logged_by_its_template(
    caplog: pytest.LogCaptureFixture,
) -> None:
    app = create_app(settings())

    async def numbered(n: int) -> dict[str, int]:
        return {"n": n}

    app.add_api_route("/test/{n}", numbered)
    caplog.set_level(logging.INFO)
    TestClient(app).get("/test/12345")
    lines = [r.getMessage() for r in caplog.records if r.name == "clearcut_worker"]
    assert len(lines) == 1
    assert lines[0].startswith("GET /test/{n} -> 200 (")
    assert "12345" not in lines[0]


# ---------- a picture as the document ----------


def test_a_png_or_a_jpeg_is_a_one_page_document(client: TestClient) -> None:
    for body, media_type in [(png(30, 20), "image/png"), (jpeg(30, 20), "image/jpeg")]:
        response = client.post(INSPECT, content=body, headers=AUTH)
        assert response.status_code == 200
        assert response.json() == {
            "media_type": media_type,
            "pages": 1,
            "pages_with_text": 0,
            "has_text_layer": False,
        }


def test_a_huge_picture_is_a_422(client: TestClient) -> None:
    response = client.post(INSPECT, content=claims_to_be(30_000, 30_000), headers=AUTH)
    assert response.status_code == 422
    assert response.json() == {"error": "the image is too large"}
