// Written by Kone Tshivhinda & Claude
//! The web page: four small files served by the server itself, with no build step and nothing
//! loaded from anywhere else.
//!
//! - `/` the page, `/app.css` its look, `/app.js` its behaviour, `/favicon.svg` the tab's mark.
//!   None of them needs a key (they
//!   hold nothing private); the page asks for the key and sends it to the API itself.
//! - **A strict content security policy**: scripts and styles only from this server, no inline
//!   script, nothing embedded, no other origin to talk to. Text that comes off an uploaded
//!   document is shown with `textContent` only (a test fails if the script ever uses
//!   `innerHTML`), so a document cannot put script in the page.

use axum::http::{header, HeaderName, HeaderValue};
use axum::response::IntoResponse;

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; \
                   img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn file(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP)),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (header::REFERRER_POLICY, HeaderValue::from_static("no-referrer")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
            (
                HeaderName::from_static("x-frame-options"),
                HeaderValue::from_static("DENY"),
            ),
        ],
        body,
    )
}

pub(crate) async fn index() -> impl IntoResponse {
    file("text/html; charset=utf-8", include_str!("web/index.html"))
}

pub(crate) async fn stylesheet() -> impl IntoResponse {
    file("text/css; charset=utf-8", include_str!("web/app.css"))
}

pub(crate) async fn script() -> impl IntoResponse {
    file("text/javascript; charset=utf-8", include_str!("web/app.js"))
}

pub(crate) async fn favicon() -> impl IntoResponse {
    file("image/svg+xml", include_str!("web/favicon.svg"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{send, FakeAuth};
    use crate::{finish, routes, AppState};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};

    async fn fetch(path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
        let app = finish(routes(), AppState::shared(FakeAuth::new()));
        send(&app, Request::get(path).body(Body::empty()).unwrap()).await
    }

    #[tokio::test]
    async fn the_page_and_its_files_are_served_without_a_key_and_locked_down() {
        for (path, kind) in [
            ("/", "text/html"),
            ("/app.css", "text/css"),
            ("/app.js", "text/javascript"),
            ("/favicon.svg", "image/svg+xml"),
        ] {
            let (status, headers, body) = fetch(path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert!(
                headers[header::CONTENT_TYPE].to_str().unwrap().starts_with(kind),
                "{path}"
            );
            let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
            assert!(
                csp.contains("default-src 'none'") && csp.contains("script-src 'self'"),
                "{csp}"
            );
            assert!(
                !csp.contains("unsafe-inline") && !csp.contains("unsafe-eval"),
                "{csp}"
            );
            assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
            assert!(!body.is_empty());
        }
    }

    #[tokio::test]
    async fn the_page_loads_only_its_own_files_and_has_no_inline_script() {
        let (_, _, html) = fetch("/").await;
        assert!(
            html.contains("src=\"/app.js\"")
                && html.contains("href=\"/app.css\"")
                && html.contains("href=\"/favicon.svg\"")
        );
        assert!(
            !html.contains("http://") && !html.contains("https://"),
            "nothing from elsewhere"
        );
        let scripts = html.matches("<script").count();
        assert_eq!(scripts, 1, "one script, and it is a file");
        assert!(
            !html.contains(" onclick=") && !html.contains(" onload="),
            "no inline handlers"
        );
    }

    #[tokio::test]
    async fn the_script_never_puts_text_from_a_document_into_the_page_as_markup() {
        let (_, _, js) = fetch("/app.js").await;
        for forbidden in [
            "innerHTML",
            "outerHTML",
            "insertAdjacentHTML",
            "document.write",
            "eval(",
            "new Function",
        ] {
            assert!(!js.contains(forbidden), "{forbidden} in app.js");
        }
        // And the key stays in memory: never written to storage or a cookie.
        for forbidden in ["localStorage", "sessionStorage", "document.cookie", "indexedDB"] {
            assert!(!js.contains(forbidden), "{forbidden} in app.js");
        }
    }
}
