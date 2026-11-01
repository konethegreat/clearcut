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

use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::response::IntoResponse;

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; \
                   img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

// Crawlers and some browsers still ask for /favicon.ico even when the page links an SVG.
const ROBOTS: &str = "User-agent: *\nAllow: /\nDisallow: /v1/\nDisallow: /health\n";

// RFC 9116. The Expires date must be rotated; a test fails if this line disappears.
const SECURITY_TXT: &str = "\
Contact: https://github.com/konethegreat/clearcut/security/advisories/new
Expires: 2027-04-30T00:00:00.000Z
Policy: https://github.com/konethegreat/clearcut/blob/main/SECURITY.md
Preferred-Languages: en
";

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

pub(crate) async fn favicon_ico() -> impl IntoResponse {
    (
        StatusCode::MOVED_PERMANENTLY,
        [(header::LOCATION, HeaderValue::from_static("/favicon.svg"))],
    )
}

pub(crate) async fn robots() -> impl IntoResponse {
    file("text/plain; charset=utf-8", ROBOTS)
}

pub(crate) async fn security_txt() -> impl IntoResponse {
    file("text/plain; charset=utf-8", SECURITY_TXT)
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
            ("/robots.txt", "text/plain"),
            ("/.well-known/security.txt", "text/plain"),
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
    async fn the_page_offers_a_demo_that_needs_no_key() {
        let (_, _, html) = fetch("/").await;
        assert!(html.contains("href=\"#/demo\""), "the header links the demo");
        let (status, _, js) = fetch("/app.js").await;
        assert_eq!(status, StatusCode::OK);
        for marker in [
            "const DEMO_DOCUMENTS",
            "const DEMO_PERIOD = \"1 March to 31 May 2026\"",
            "Sample documents. Nothing here is saved.",
            "Kloof Street Coffee Roasters",
            "March to May 2026",
            "Eighteen supplier documents",
            "#/demo/invoice/",
            "#\\/demo\\/(invoice|document)",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
    }

    #[tokio::test]
    async fn the_demo_keeps_its_prepared_arithmetic() {
        let (_, _, js) = fetch("/app.js").await;
        for marker in [
            "const DEMO_AUTOMATION_BPS = 7650",
            "net: 4821649",
            "vat: 723247",
            "gross: 5544896",
            "grossRead: 1250100",
            "grossPrinted: 1250000",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
    }

    #[tokio::test]
    async fn the_demo_workspace_and_queue_match_the_quarter() {
        let (_, _, js) = fetch("/app.js").await;
        for marker in [
            "function demoWorkspace",
            "function demoQueue",
            "\"#/demo/workspace\"",
            "\"#/demo/review\"",
            "Open the workspace",
            "See what needs a person",
            "Nothing is waiting for a person.",
            "Review this document",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
    }

    #[tokio::test]
    async fn the_demo_shows_every_kind_of_review() {
        let (_, _, js) = fetch("/app.js").await;
        for marker in [
            "The reading says R 12 501.00",
            "A foreign invoice in US dollars",
            "Line 37 is 3 × R 412.50",
            "This invoice has no date",
            "Keep it in US dollars",
            "Accept the printed line",
            "Use 12 May 2026",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
    }

    #[tokio::test]
    async fn the_demo_cannot_reach_the_api_or_storage() {
        let (_, _, js) = fetch("/app.js").await;
        let start = js
            .find("// ---------------------------------------------------------------- 1b. the demo ----")
            .expect("the demo marker");
        let end = js
            .find("// ---------------------------------------------------------------- bulk intake ----")
            .expect("the bulk intake marker");
        let demo = &js[start..end];
        for forbidden in [
            "fetch(",
            "api(",
            "Authorization",
            "localStorage",
            "sessionStorage",
            "indexedDB",
            "document.cookie",
        ] {
            assert!(!demo.contains(forbidden), "{forbidden} in the demo");
        }
    }

    #[tokio::test]
    async fn the_register_screen_is_reachable_and_says_what_it_is() {
        let (_, _, html) = fetch("/").await;
        assert!(
            html.contains("id=\"register-link\"") && html.contains("href=\"#/register\""),
            "the header links the register"
        );
        let (_, _, js) = fetch("/app.js").await;
        for marker in [
            "function registerPage",
            "const PERIODS",
            "Last month",
            "#/register?ids=",
            "clearcut-register",
            "The file is built when you ask for it; nothing is kept.",
            "Check the file against the originals",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
        let (_, _, css) = fetch("/app.css").await;
        assert!(
            css.contains("input[type=\"date\"]"),
            "the custom range's dates are styled"
        );
    }

    #[tokio::test]
    async fn the_icon_request_is_redirected_to_the_svg() {
        let (status, headers, _) = fetch("/favicon.ico").await;
        assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
        assert_eq!(headers[header::LOCATION], "/favicon.svg");
    }

    #[tokio::test]
    async fn the_records_crawl_and_security_readers_look_for_are_present() {
        let (status, _, robots) = fetch("/robots.txt").await;
        assert_eq!(status, StatusCode::OK);
        assert!(robots.contains("Allow: /"));
        assert!(robots.contains("Disallow: /v1/"), "{robots}");

        let (status, _, security) = fetch("/.well-known/security.txt").await;
        assert_eq!(status, StatusCode::OK);
        for line in [
            "Contact: https://github.com/konethegreat/clearcut/security/advisories/new",
            "Expires: 2027-04-30T00:00:00.000Z",
            "Policy: https://github.com/konethegreat/clearcut/blob/main/SECURITY.md",
        ] {
            assert!(security.contains(line), "{line} in security.txt");
        }
        assert!(
            !security.contains("Canonical"),
            "no host that will move when the deployment does"
        );
    }

    #[tokio::test]
    async fn the_site_names_its_owner_and_what_it_does_with_a_document() {
        let (status, _, js) = fetch("/app.js").await;
        assert_eq!(status, StatusCode::OK);
        for marker in [
            "href: \"#/trust\"",
            "function trustPage",
            "Copyright 2026 Kone Tshivhinda",
            "Apache-2.0",
            "provided as is",
            "not tax, legal or accounting advice",
            "GitHub Security Advisories",
            "separated by the database itself",
            "erases the file and everything read from it",
            "365 days by default",
            "no analytics",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
        let route = js.find("hash === \"#/trust\"").expect("a trust route");
        let sign_in = js.find("if (!key)").expect("the key check");
        assert!(route < sign_in, "the trust page is routed before the key check");
    }

    #[tokio::test]
    async fn every_screen_says_the_software_is_still_alpha() {
        let (_, _, html) = fetch("/").await;
        assert!(html.contains("class=\"alpha\""), "the banner is in the page");
        assert!(html.contains("<strong>Alpha</strong>"));
        assert!(
            html.contains("ClearCut is in active development."),
            "it says plainly what alpha means"
        );
        // Static markup above the app shell, so it shows on the demo and every route.
        let banner = html.find("class=\"alpha\"").unwrap();
        let shell = html.find("<header class=\"bar\"").unwrap();
        assert!(banner < shell, "the banner comes before the app shell");
        let (.., css) = fetch("/app.css").await;
        assert!(css.contains(".alpha"), "the banner has its own styles");
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

    #[tokio::test]
    async fn the_upload_screen_takes_a_batch_and_says_what_it_costs() {
        let (_, _, js) = fetch("/app.js").await;
        for marker in [
            "function bulkCard",
            "up to 20 invoices",
            "Each document is sent to Anthropic to be read and costs money.",
            "multiple: true",
            "Try again",
            "Still reading",
            "Wait for it to finish, then choose the rest.",
        ] {
            assert!(js.contains(marker), "{marker} in app.js");
        }
        // The batch path must not reintroduce markup from a file name or the server.
        for forbidden in ["innerHTML", "outerHTML", "insertAdjacentHTML"] {
            assert!(!js.contains(forbidden), "{forbidden} in app.js");
        }
    }
}
