//! `humpyard ui` (spec: `cli`, `admin-gui`): the forwarding, the checks that defeat rebinding and cross-site
//! requests, the access token, and the credential's confinement, against a fake gateway that records what reaches it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use common::serve;
use humpyard::ui::{Prepared, Settings, prepare, prepare_with_timeout};
use parking_lot::Mutex;
use tower::ServiceExt;

const CREDENTIAL: &str = "adm-test-credential-5521";

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    uri: String,
    authorization: Option<String>,
    cookie: Option<String>,
    body: Vec<u8>,
}

type Log = Arc<Mutex<Vec<Seen>>>;

/// A gateway that records every request and answers by path.
async fn gateway() -> (String, Log) {
    let log: Log = Arc::default();
    let record = log.clone();
    let app = Router::new().fallback(move |request: Request| {
        let record = record.clone();
        async move {
            let (parts, body) = request.into_parts();
            let body = axum::body::to_bytes(body, usize::MAX)
                .await
                .unwrap()
                .to_vec();
            let header_of = |name| {
                parts
                    .headers
                    .get(name)
                    .map(|v| v.to_str().unwrap().to_string())
            };
            let authorization = header_of(header::AUTHORIZATION);
            record.lock().push(Seen {
                method: parts.method.to_string(),
                uri: parts.uri.to_string(),
                authorization: authorization.clone(),
                cookie: header_of(header::COOKIE),
                body,
            });
            match parts.uri.path() {
                "/admin/v1/redirect" => Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::LOCATION, "/elsewhere")
                    .body(Body::empty())
                    .unwrap(),
                "/admin/v1/big" => vec![b'x'; 9 * 1024 * 1024].into_response(),
                "/admin/v1/slow" => {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    "late".into_response()
                }
                // A gateway that echoes what it was sent, which the command must not pass on.
                "/admin/v1/echo" => authorization.unwrap_or_default().into_response(),
                "/admin/v1/leaky-header" => Response::builder()
                    .header("x-debug", authorization.unwrap_or_default())
                    .body(Body::from("{}"))
                    .unwrap(),
                _ => axum::Json(serde_json::json!({"ok": true, "path": parts.uri.path()}))
                    .into_response(),
            }
        }
    });
    (format!("http://{}", serve(app).await), log)
}

fn settings(url: &str) -> Settings {
    Settings {
        url: url.into(),
        listen: "127.0.0.1:8787".into(),
        credential_env: None,
        allow_remote_with_credential: false,
        allow_hosts: Vec::new(),
        allow_insecure_upstream: false,
    }
}

fn env(name: &str) -> Option<String> {
    (name == "ADMIN_CREDENTIAL").then(|| CREDENTIAL.to_string())
}

fn with_credential(url: &str) -> Settings {
    Settings {
        credential_env: Some("ADMIN_CREDENTIAL".into()),
        ..settings(url)
    }
}

fn start(settings: &Settings) -> Prepared {
    prepare(settings, &env).unwrap()
}

struct Reply {
    status: StatusCode,
    headers: header::HeaderMap,
    body: String,
}

async fn ask(
    app: &Router,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost:8787");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    Reply {
        status: parts.status,
        headers: parts.headers,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

async fn get(app: &Router, uri: &str) -> Reply {
    ask(app, Method::GET, uri, &[], "").await
}

fn header_of<'a>(reply: &'a Reply, name: &str) -> &'a str {
    reply.headers.get(name).map_or("", |v| v.to_str().unwrap())
}

// ---------------------------------------------------------------- forwarding

#[tokio::test]
async fn the_admin_api_is_forwarded_with_its_query_and_the_answer_comes_back() {
    let (url, log) = gateway().await;
    let app = start(&settings(&url)).router;
    let reply = ask(
        &app,
        Method::GET,
        "/admin/v1/stats?window=7d&group_by=target",
        &[("authorization", "Bearer browser-own")],
        "",
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert!(
        reply.body.contains("\"path\":\"/admin/v1/stats\""),
        "{}",
        reply.body
    );
    let seen = log.lock().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].uri, "/admin/v1/stats?window=7d&group_by=target");
    // With no credential of its own, the browser's is passed on.
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer browser-own"));
}

#[tokio::test]
async fn a_post_body_is_forwarded_and_cookies_never_are() {
    let (url, log) = gateway().await;
    let app = start(&settings(&url)).router;
    let reply = ask(
        &app,
        Method::POST,
        "/admin/v1/config/patch",
        &[
            ("content-type", "application/json"),
            ("cookie", "humpyard_ui=abc; other=1"),
        ],
        "{\"edits\":[]}",
    )
    .await;
    assert_eq!(reply.status, 200);
    let seen = log.lock().clone();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].body, b"{\"edits\":[]}");
    assert_eq!(seen[0].cookie, None);
}

#[tokio::test]
async fn nothing_but_the_admin_api_is_forwarded() {
    let (url, log) = gateway().await;
    let app = start(&settings(&url)).router;
    for uri in [
        "/v1/chat/completions",
        "/v1/models",
        "/admin/ui/",
        "/admin/v1",
        "/admin",
    ] {
        let reply = get(&app, uri).await;
        assert_eq!(reply.status, 404, "{uri}");
    }
    // The gateway's other paths are not forwarded either; a path with no extension is a route of the page.
    for uri in ["/health", "/metrics"] {
        assert_eq!(get(&app, uri).await.status, 200, "{uri}");
    }
    assert!(log.lock().is_empty());
}

#[tokio::test]
async fn a_path_that_leaves_the_prefix_is_refused_in_every_encoding() {
    let (url, log) = gateway().await;
    let app = start(&settings(&url)).router;
    for uri in [
        "/admin/v1/../v1/chat/completions",
        "/admin/v1/%2e%2e/v1/chat/completions",
        "/admin/v1/%2E%2E/v1/chat/completions",
        "/admin/v1/.%2e/v1/chat/completions",
        "/admin/v1/..%2fv1/chat/completions",
        "/admin/v1/..%5cv1",
        "/admin/v1/./health",
        "/admin/v1/health/..",
        "/../../etc/passwd",
        "/%2e%2e/%2e%2e/etc/passwd",
        "/assets/..%2f..%2fCargo.toml",
    ] {
        assert_eq!(get(&app, uri).await.status, 404, "{uri}");
    }
    assert!(log.lock().is_empty());
}

#[tokio::test]
async fn repeated_slashes_collapse_and_stay_under_the_prefix() {
    let (url, log) = gateway().await;
    let app = start(&settings(&url)).router;
    assert_eq!(get(&app, "/admin//v1///health").await.status, 200);
    assert_eq!(log.lock()[0].uri, "/admin/v1/health");
}

#[tokio::test]
async fn a_redirect_is_returned_and_not_followed() {
    let (url, log) = gateway().await;
    let app = start(&settings(&url)).router;
    let reply = get(&app, "/admin/v1/redirect").await;
    assert_eq!(reply.status, 302);
    assert_eq!(header_of(&reply, "location"), "/elsewhere");
    assert_eq!(log.lock().len(), 1, "the redirect target was not requested");
}

#[tokio::test]
async fn an_oversized_answer_and_an_oversized_request_are_refused() {
    let (url, _log) = gateway().await;
    let app = start(&settings(&url)).router;
    let reply = get(&app, "/admin/v1/big").await;
    assert_eq!(reply.status, 502);
    assert!(
        reply.body.contains("ui_upstream_too_large"),
        "{}",
        reply.body
    );
    let huge = "x".repeat(300 * 1024);
    let reply = ask(&app, Method::POST, "/admin/v1/config/patch", &[], &huge).await;
    assert_eq!(reply.status, 413);
}

#[tokio::test]
async fn a_slow_gateway_is_given_up_on() {
    let (url, _log) = gateway().await;
    let app = prepare_with_timeout(&settings(&url), &env, Duration::from_millis(300))
        .unwrap()
        .router;
    let reply = get(&app, "/admin/v1/slow").await;
    assert_eq!(reply.status, 504);
    assert!(reply.body.contains("ui_upstream_timeout"), "{}", reply.body);
}

#[tokio::test]
async fn an_unreachable_gateway_is_reported_without_its_address() {
    let app = start(&settings("http://127.0.0.1:9")).router;
    let reply = get(&app, "/admin/v1/health").await;
    assert_eq!(reply.status, 502);
    assert!(!reply.body.contains("127.0.0.1"), "{}", reply.body);
}

#[tokio::test]
async fn methods_the_admin_api_does_not_use_are_refused() {
    let (url, log) = gateway().await;
    let app = start(&settings(&url)).router;
    assert_eq!(
        ask(&app, Method::PUT, "/admin/v1/overrides", &[], "{}")
            .await
            .status,
        405
    );
    assert_eq!(
        ask(&app, Method::PATCH, "/admin/v1/overrides", &[], "{}")
            .await
            .status,
        405
    );
    assert!(log.lock().is_empty());
}

// ------------------------------------------------------- hosts and origins

#[tokio::test]
async fn a_rebound_host_name_is_refused_and_nothing_is_forwarded() {
    let (url, log) = gateway().await;
    let app = start(&settings(&url)).router;
    let request = Request::builder()
        .uri("/admin/v1/health")
        .header(header::HOST, "attacker.example:8787")
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), 403);
    let none = Request::builder()
        .uri("/admin/v1/health")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(none).await.unwrap().status(),
        403,
        "no Host header at all"
    );
    assert!(log.lock().is_empty());
}

#[tokio::test]
async fn a_cross_site_request_is_refused() {
    let (url, log) = gateway().await;
    let app = start(&settings(&url)).router;
    let reply = ask(
        &app,
        Method::GET,
        "/admin/v1/health",
        &[("sec-fetch-site", "cross-site")],
        "",
    )
    .await;
    assert_eq!(reply.status, 403);
    let reply = ask(
        &app,
        Method::GET,
        "/",
        &[("sec-fetch-site", "cross-site")],
        "",
    )
    .await;
    assert_eq!(reply.status, 403);
    let reply = ask(
        &app,
        Method::GET,
        "/admin/v1/health",
        &[("sec-fetch-site", "same-origin")],
        "",
    )
    .await;
    assert_eq!(reply.status, 200);
    assert_eq!(log.lock().len(), 1);
}

#[tokio::test]
async fn a_state_changing_request_from_another_origin_is_refused() {
    let (url, log) = gateway().await;
    let app = start(&settings(&url)).router;
    let foreign = ask(
        &app,
        Method::POST,
        "/admin/v1/overrides",
        &[("origin", "http://attacker.example")],
        "{}",
    )
    .await;
    assert_eq!(foreign.status, 403);
    let null = ask(
        &app,
        Method::POST,
        "/admin/v1/overrides",
        &[("origin", "null")],
        "{}",
    )
    .await;
    assert_eq!(null.status, 403);
    assert!(log.lock().is_empty());
    let own = ask(
        &app,
        Method::POST,
        "/admin/v1/overrides",
        &[("origin", "http://localhost:8787")],
        "{}",
    )
    .await;
    assert_eq!(own.status, 200);
    // A read carrying a foreign Origin is not state-changing.
    let read = ask(
        &app,
        Method::GET,
        "/admin/v1/health",
        &[("origin", "http://attacker.example")],
        "",
    )
    .await;
    assert_eq!(read.status, 200);
}

#[tokio::test]
async fn a_named_host_is_answered_and_any_other_is_not() {
    let (url, _log) = gateway().await;
    let prepared = start(&Settings {
        listen: "0.0.0.0:8787".into(),
        allow_hosts: vec!["ui.internal:8787".into()],
        ..settings(&url)
    });
    let at = |host: &str| {
        Request::builder()
            .uri("/admin/v1/health")
            .header(header::HOST, host.to_string())
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        prepared
            .router
            .clone()
            .oneshot(at("ui.internal:8787"))
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        prepared
            .router
            .clone()
            .oneshot(at("other.internal:8787"))
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        prepared
            .router
            .clone()
            .oneshot(at("localhost:8787"))
            .await
            .unwrap()
            .status(),
        200
    );
}

// ----------------------------------------------------- starting safely

fn refusal(settings: &Settings) -> String {
    match prepare(settings, &env) {
        Ok(_) => panic!("started"),
        Err(message) => message,
    }
}

#[test]
fn a_public_address_with_no_allowed_host_does_not_start() {
    let message = refusal(&Settings {
        listen: "0.0.0.0:8787".into(),
        ..settings("https://gateway.example")
    });
    assert!(message.contains("--allow-host"), "{message}");
}

#[test]
fn a_credential_with_a_public_address_does_not_start_without_saying_so() {
    let base = Settings {
        listen: "0.0.0.0:8787".into(),
        allow_hosts: vec!["ui.internal".into()],
        ..with_credential("https://gateway.example")
    };
    let message = refusal(&base);
    assert!(
        message.contains("--allow-remote-with-credential"),
        "{message}"
    );
    assert!(!message.contains(CREDENTIAL));
    let allowed = Settings {
        allow_remote_with_credential: true,
        ..base
    };
    assert!(prepare(&allowed, &env).is_ok());
}

#[test]
fn a_credential_is_not_sent_in_clear_text_to_another_machine() {
    let message = refusal(&with_credential("http://gateway.example"));
    assert!(message.contains("https://"), "{message}");
    assert!(!message.contains(CREDENTIAL));
    let allowed = Settings {
        allow_insecure_upstream: true,
        ..with_credential("http://gateway.example")
    };
    assert!(prepare(&allowed, &env).is_ok());
}

#[test]
fn a_credential_may_go_in_clear_text_to_loopback_and_the_upstream_is_named() {
    let prepared = prepare(&with_credential("http://127.0.0.1:8080"), &env).unwrap();
    assert_eq!(prepared.upstream, "http://127.0.0.1:8080");
    let prepared = prepare(&with_credential("http://localhost:8080"), &env).unwrap();
    assert_eq!(prepared.upstream, "http://localhost:8080");
}

#[test]
fn a_missing_credential_variable_and_a_bad_address_are_reported() {
    let message = refusal(&Settings {
        credential_env: Some("NOPE".into()),
        ..settings("https://g.example")
    });
    assert!(message.contains("NOPE"), "{message}");
    for bad in [
        "gateway.example",
        "ftp://g.example",
        "https://user:pw@g.example",
        "https://g.example/path",
        "https://g.example/?q=1",
    ] {
        assert!(prepare(&settings(bad), &env).is_err(), "{bad}");
    }
    assert!(
        prepare(
            &Settings {
                listen: "not-an-address".into(),
                ..settings("https://g.example")
            },
            &env
        )
        .is_err()
    );
    assert!(
        prepare(
            &Settings {
                allow_hosts: vec!["a b".into()],
                ..settings("https://g.example")
            },
            &env
        )
        .is_err()
    );
}

// --------------------------------------------- the credential and the token

#[tokio::test]
async fn without_a_held_credential_no_token_is_asked_for() {
    let (url, _log) = gateway().await;
    let prepared = start(&settings(&url));
    assert!(prepared.token.is_none());
    assert_eq!(get(&prepared.router, "/admin/v1/health").await.status, 200);
    assert_eq!(
        ask(&prepared.router, Method::POST, "/ui/session", &[], "{}")
            .await
            .status,
        404
    );
}

#[tokio::test]
async fn with_a_held_credential_the_data_path_needs_the_session_cookie() {
    let (url, log) = gateway().await;
    let prepared = start(&with_credential(&url));
    let token = prepared.token.clone().unwrap();
    assert_eq!(token.len(), 64);
    let app = prepared.router;

    let reply = get(&app, "/admin/v1/health").await;
    assert_eq!(reply.status, 401);
    assert!(reply.body.contains("ui_token_required"), "{}", reply.body);
    assert!(log.lock().is_empty(), "nothing was forwarded");
    // The page itself needs no token: it is what asks for it.
    assert_eq!(get(&app, "/").await.status, 200);

    let wrong = ask(
        &app,
        Method::POST,
        "/ui/session",
        &[],
        "{\"token\":\"nope\"}",
    )
    .await;
    assert_eq!(wrong.status, 401);
    assert!(wrong.headers.get(header::SET_COOKIE).is_none());
    let in_url = get(&app, &format!("/ui/session?token={token}")).await;
    assert_eq!(in_url.status, 405, "a token in a URL is not accepted");
    assert!(log.lock().is_empty());
}

#[tokio::test]
async fn the_token_is_exchanged_for_a_cookie_that_opens_the_data_path_with_the_held_credential() {
    let (url, log) = gateway().await;
    let prepared = start(&with_credential(&url));
    let token = prepared.token.clone().unwrap();
    let app = prepared.router;

    let exchange = ask(
        &app,
        Method::POST,
        "/ui/session",
        &[("content-type", "application/json")],
        &format!("{{\"token\":\"{token}\"}}"),
    )
    .await;
    assert_eq!(exchange.status, 204);
    let cookie = header_of(&exchange, "set-cookie").to_string();
    assert!(
        cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"),
        "{cookie}"
    );
    assert!(!cookie.contains(&token), "the cookie is not the token");
    let pair = cookie.split(';').next().unwrap().to_string();

    // The browser's own Authorization is replaced by the held credential.
    let reply = ask(
        &app,
        Method::GET,
        "/admin/v1/health",
        &[("cookie", &pair), ("authorization", "Bearer browser-own")],
        "",
    )
    .await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    let seen = log.lock().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].authorization, Some(format!("Bearer {CREDENTIAL}")));
    assert_eq!(seen[0].cookie, None, "the session cookie stays here");
    assert!(!reply.body.contains(CREDENTIAL));
}

#[tokio::test]
async fn a_gateway_that_echoes_the_credential_is_not_passed_on() {
    let (url, _log) = gateway().await;
    let prepared = start(&with_credential(&url));
    let token = prepared.token.clone().unwrap();
    let app = prepared.router;
    let exchange = ask(
        &app,
        Method::POST,
        "/ui/session",
        &[],
        &format!("{{\"token\":\"{token}\"}}"),
    )
    .await;
    let cookie = header_of(&exchange, "set-cookie")
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let reply = ask(
        &app,
        Method::GET,
        "/admin/v1/echo",
        &[("cookie", &cookie)],
        "",
    )
    .await;
    assert_eq!(reply.status, 502);
    assert!(!reply.body.contains(CREDENTIAL));
    // A header the command does not pass on never reaches the browser, so an echo in one cannot leak.
    let reply = ask(
        &app,
        Method::GET,
        "/admin/v1/leaky-header",
        &[("cookie", &cookie)],
        "",
    )
    .await;
    assert_eq!(reply.status, 200);
    assert!(reply.headers.get("x-debug").is_none());
    assert!(
        reply
            .headers
            .values()
            .all(|v| !v.to_str().unwrap().contains(CREDENTIAL))
    );
}

#[tokio::test]
async fn the_start_up_text_shows_the_token_and_never_the_credential() {
    let (url, _log) = gateway().await;
    let (halt, halted) = tokio::sync::oneshot::channel::<()>();
    let settings = Settings {
        listen: "127.0.0.1:0".into(),
        ..with_credential(&url)
    };
    let mut out = Vec::new();
    let shutdown = async move {
        let _ = halted.await;
    };
    let serving = async { humpyard::ui::run(&settings, &env, &mut out, shutdown).await };
    let stopper = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        halt.send(()).unwrap();
    };
    let (result, ()) = tokio::join!(serving, stopper);
    result.unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("access token"), "{text}");
    assert!(text.contains("ADMIN_CREDENTIAL"), "{text}");
    assert!(!text.contains(CREDENTIAL), "{text}");
    assert!(text.contains(&url), "the upstream is named: {text}");
}

#[test]
fn a_secret_cannot_be_printed_by_accident() {
    // `Prepared` has no Debug; the secret type redacts, so a stray `{:?}` of anything holding it shows nothing.
    let prepared = start(&with_credential("https://gateway.example"));
    assert_eq!(prepared.credential_env.as_deref(), Some("ADMIN_CREDENTIAL"));
    assert!(!format!("{:?}", prepared.token).contains(CREDENTIAL));
}

// ------------------------------------------------------------ the files

#[tokio::test]
async fn every_response_carries_the_restrictive_headers() {
    let (url, _log) = gateway().await;
    let app = start(&settings(&url)).router;
    for (uri, status) in [
        ("/", 200),
        ("/models", 200),
        ("/assets/nope.js", 404),
        ("/v1/models", 404),
        ("/admin/v1/health", 200),
    ] {
        let reply = get(&app, uri).await;
        assert_eq!(reply.status, status, "{uri}");
        let policy = header_of(&reply, "content-security-policy");
        for part in [
            "default-src 'self'",
            "script-src 'self'",
            "style-src 'self'",
            "connect-src 'self'",
            "frame-ancestors 'none'",
            "base-uri 'none'",
        ] {
            assert!(policy.contains(part), "{uri}: {policy}");
        }
        assert!(
            !policy.contains("unsafe-inline") && !policy.contains("unsafe-eval"),
            "{uri}: {policy}"
        );
        assert_eq!(
            header_of(&reply, "x-content-type-options"),
            "nosniff",
            "{uri}"
        );
        assert_eq!(header_of(&reply, "referrer-policy"), "no-referrer", "{uri}");
    }
    let refused = Request::builder()
        .uri("/")
        .header(header::HOST, "evil.example")
        .body(Body::empty())
        .unwrap();
    let refused = app.oneshot(refused).await.unwrap();
    assert_eq!(refused.status(), 403);
    assert!(
        refused.headers().contains_key("content-security-policy"),
        "a refusal is secured too"
    );
}

#[tokio::test]
async fn the_page_is_served_without_a_credential_and_a_client_route_loads_it() {
    let (url, _log) = gateway().await;
    let app = start(&settings(&url)).router;
    let root = get(&app, "/").await;
    assert_eq!(root.status, 200);
    assert!(header_of(&root, "content-type").starts_with("text/html"));
    assert_eq!(header_of(&root, "cache-control"), "no-cache");
    let route = get(&app, "/models").await;
    assert_eq!(route.status, 200);
    assert_eq!(
        route.body, root.body,
        "a client-side route loads the entry page"
    );
    assert_eq!(get(&app, "/assets/nope.js").await.status, 404);
    assert_eq!(get(&app, "/favicon.ico").await.status, 404);
    assert_eq!(ask(&app, Method::POST, "/", &[], "x").await.status, 405);
}

#[cfg(not(feature = "ui"))]
#[tokio::test]
async fn a_build_without_the_gui_serves_a_placeholder_that_says_how_to_build_it() {
    let (url, _log) = gateway().await;
    let app = start(&settings(&url)).router;
    let root = get(&app, "/").await;
    assert!(root.body.contains("not in this build"), "{}", root.body);
    assert!(root.body.contains("npm run build"), "{}", root.body);
}

#[cfg(feature = "ui")]
#[tokio::test]
async fn a_build_with_the_gui_serves_its_page_and_hashed_files_as_immutable() {
    let (url, _log) = gateway().await;
    let app = start(&settings(&url)).router;
    let root = get(&app, "/").await;
    assert!(root.body.contains("id=\"app\""), "{}", root.body);
    assert!(!root.body.contains("not in this build"));
    // The page's own script and style are served, immutable, with their types.
    let script = root
        .body
        .split("src=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    let reply = get(&app, &script).await;
    assert_eq!(reply.status, 200, "{script}");
    assert!(header_of(&reply, "content-type").starts_with("text/javascript"));
    assert!(header_of(&reply, "cache-control").contains("immutable"));
}
