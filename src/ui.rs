//! `humpyard ui`: serves the admin GUI and forwards `/admin/v1` to a gateway (ADR 0032, spec: `cli`, `admin-gui`).
//!
//! It is a client command like `ctl`: the serving path never reaches it, so a deployment that does not run it has
//! no web page and no static-file path. A local web service is reachable by any page the operator visits, so every
//! request is checked before it is looked at: its `Host` must name this machine (a rebound DNS name is refused),
//! a cross-site request is refused, and when the command holds the admin credential a per-run access token stands
//! between the credential and anyone else who can reach the port. Only the `ui` command (`main`) may use this
//! module.

use std::future::Future;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::Response;
use reqwest::Url;

mod forward;
mod guard;
mod path;

use forward::Upstream;
use guard::Sessions;

/// The path the access token is exchanged on. It is outside `/admin/v1`, and the token travels in the body.
const SESSION_PATH: &str = "/ui/session";

/// What the operator typed, before it is checked.
pub struct Settings {
    pub url: String,
    pub listen: String,
    pub credential_env: Option<String>,
    pub allow_remote_with_credential: bool,
    pub allow_hosts: Vec<String>,
    pub allow_insecure_upstream: bool,
}

/// A secret that cannot be printed by accident: formatting it shows nothing of it.
pub(crate) struct Secret(String);

impl Secret {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// The checked service: the router to serve, where to serve it, and what to tell the operator.
pub struct Prepared {
    pub router: Router,
    pub listen: SocketAddr,
    /// Where requests go, as printed at start: scheme, host and port only.
    pub upstream: String,
    /// The environment variable the credential came from, when it is injected.
    pub credential_env: Option<String>,
    /// The access token, when the credential is injected. Shown once and never again.
    pub token: Option<String>,
}

struct Shared {
    upstream: Upstream,
    allow_hosts: Vec<String>,
    /// Present exactly when the command injects the admin credential.
    sessions: Option<Sessions>,
}

fn is_loopback_host(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    })
}

fn parse_upstream(address: &str) -> Result<Url, String> {
    let url = Url::parse(address)
        .map_err(|e| format!("`{address}` is not an address like https://host: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "the address scheme must be http or https, not {}",
            url.scheme()
        ));
    }
    if url.host_str().is_none() {
        return Err(format!("`{address}` names no host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("the address must not carry a user name or password".into());
    }
    if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
        return Err(
            "the address must be only a scheme, a host and a port, like https://gateway.example"
                .into(),
        );
    }
    Ok(url)
}

/// Checks the settings and builds the service. Every refusal here is a reason to not start, in words the operator
/// can act on; none of them quotes the credential.
///
/// # Errors
/// The first setting that is not safe or not usable.
pub fn prepare(
    settings: &Settings,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Prepared, String> {
    prepare_with_timeout(settings, env, forward::TIMEOUT)
}

/// [`prepare`] with another bound on how long a forwarded request may take, for tests of that bound.
///
/// # Errors
/// As [`prepare`].
#[doc(hidden)]
pub fn prepare_with_timeout(
    settings: &Settings,
    env: &dyn Fn(&str) -> Option<String>,
    timeout: std::time::Duration,
) -> Result<Prepared, String> {
    let listen: SocketAddr = settings.listen.parse().map_err(|e| {
        format!(
            "`{}` is not an address like 127.0.0.1:8787: {e}",
            settings.listen
        )
    })?;
    let upstream = parse_upstream(&settings.url)?;
    for host in &settings.allow_hosts {
        if host.is_empty() || host.contains(['/', ' ', '@']) {
            return Err(format!(
                "--allow-host `{host}` is not a host name like ui.internal or ui.internal:8787"
            ));
        }
    }
    if !listen.ip().is_loopback() && settings.allow_hosts.is_empty() {
        return Err(format!(
            "{listen} is not a loopback address, and a page on another site could then reach this service through a \
             DNS name it controls. Name the host people will use with --allow-host, or listen on 127.0.0.1"
        ));
    }

    let credential = match &settings.credential_env {
        None => None,
        Some(name) => {
            let value = env(name)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| format!("the environment variable {name} is not set or is empty"))?;
            if HeaderValue::from_str(&format!("Bearer {value}")).is_err() {
                return Err(format!(
                    "the value of {name} cannot be sent as an Authorization header"
                ));
            }
            if !listen.ip().is_loopback() && !settings.allow_remote_with_credential {
                return Err(format!(
                    "--credential-env makes this service act with the admin credential for anyone who can reach \
                     {listen}. Listen on 127.0.0.1, or add --allow-remote-with-credential if that is what you mean"
                ));
            }
            if upstream.scheme() != "https"
                && !is_loopback_host(&upstream)
                && !settings.allow_insecure_upstream
            {
                return Err(format!(
                    "--credential-env would send the admin credential in clear text to {}. Use an https:// address, \
                     or add --allow-insecure-upstream if the network between is trusted",
                    upstream.host_str().unwrap_or_default()
                ));
            }
            Some(Secret(value))
        }
    };

    let token = match credential {
        Some(_) => Some(new_token()?),
        None => None,
    };
    let label = {
        let mut label = format!(
            "{}://{}",
            upstream.scheme(),
            upstream.host_str().unwrap_or_default()
        );
        if let Some(port) = upstream.port() {
            label = format!("{label}:{port}");
        }
        label
    };
    let shared = Shared {
        sessions: token.as_deref().map(Sessions::new),
        upstream: Upstream::new(upstream, credential, timeout)?,
        allow_hosts: settings.allow_hosts.clone(),
    };
    let router = Router::new().fallback(handle).with_state(Arc::new(shared));
    Ok(Prepared {
        router,
        listen,
        upstream: label,
        credential_env: settings.credential_env.clone(),
        token,
    })
}

fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| format!("cannot make an access token: {e}"))?;
    Ok(hex::encode(bytes))
}

/// Checks the settings, tells the operator what it will do, and serves until `shutdown` completes.
///
/// # Errors
/// A setting that is not safe, an address that cannot be bound, or a failure while serving.
pub async fn run(
    settings: &Settings,
    env: &dyn Fn(&str) -> Option<String>,
    out: &mut dyn Write,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    let prepared = prepare(settings, env)?;
    let listener = tokio::net::TcpListener::bind(prepared.listen)
        .await
        .map_err(|e| format!("cannot bind {}: {e}", prepared.listen))?;
    let bound = listener.local_addr().map_err(|e| e.to_string())?;
    let mut say = |line: String| writeln!(out, "{line}").map_err(|e| e.to_string());
    say(format!("humpyard ui: the admin GUI is at http://{bound}"))?;
    if !crate::ui_assets::EMBEDDED {
        say("this build has no GUI files: the page says how to build them".into())?;
    }
    say(format!(
        "humpyard ui: /admin/v1 is forwarded to {}",
        prepared.upstream
    ))?;
    if let (Some(name), Some(token)) = (&prepared.credential_env, &prepared.token) {
        say(format!(
            "the admin credential in ${name} is added to forwarded requests, and sent only to {}",
            prepared.upstream
        ))?;
        say(format!(
            "access token (shown once; the page asks for it): {token}"
        ))?;
    }
    out.flush().map_err(|e| e.to_string())?;
    axum::serve(listener, prepared.router)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|e| e.to_string())
}

/// A refusal in the admin API's own error shape, so the GUI shows the reason the same way for every source.
fn json_error(status: StatusCode, code: &str, message: &str) -> Response {
    let body = serde_json::json!({"error": {"message": message, "type": "invalid_request_error", "code": code}});
    let mut response = Response::new(Body::from(body.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

/// Headers every response carries: the page may load scripts, styles and data only from its own origin, may not be
/// framed, and is not sniffed or sent anywhere as a referrer.
fn secured(mut response: Response) -> Response {
    const POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; \
                          frame-ancestors 'none'; base-uri 'none'";
    let headers = response.headers_mut();
    for (name, value) in [
        ("content-security-policy", POLICY),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
    ] {
        headers.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    response
}

async fn handle(State(shared): State<Arc<Shared>>, request: Request) -> Response {
    secured(answer(&shared, request).await)
}

/// Every check that does not need the path, in the order that matters: who is asking comes before what for.
fn refused(shared: &Shared, request: &Request) -> Option<Response> {
    let headers = request.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !guard::host_allowed(host, &shared.allow_hosts) {
        return Some(json_error(
            StatusCode::FORBIDDEN,
            "ui_host_refused",
            "This host name is not one this service answers to.",
        ));
    }
    let cross_site =
        headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) == Some("cross-site");
    if cross_site {
        return Some(json_error(
            StatusCode::FORBIDDEN,
            "ui_cross_site",
            "Cross-site requests are not answered.",
        ));
    }
    let reads = matches!(*request.method(), Method::GET | Method::HEAD);
    if !reads
        && let Some(origin) = headers.get(header::ORIGIN)
        && !origin
            .to_str()
            .is_ok_and(|origin| guard::origin_is_own(origin, host))
    {
        return Some(json_error(
            StatusCode::FORBIDDEN,
            "ui_cross_site",
            "This request came from another origin.",
        ));
    }
    None
}

async fn answer(shared: &Shared, request: Request) -> Response {
    if let Some(refusal) = refused(shared, &request) {
        return refusal;
    }
    let Some(segments) = path::segments(request.uri().path()) else {
        return not_found();
    };
    if request.uri().path() == SESSION_PATH {
        return exchange(shared, request).await;
    }
    if segments.len() > 2 && segments[0] == "admin" && segments[1] == "v1" {
        return api(shared, request, &segments[2..]).await;
    }
    if segments
        .first()
        .is_some_and(|first| first == "admin" || first == "v1")
    {
        return not_found();
    }
    asset(request.method(), &segments)
}

fn not_found() -> Response {
    json_error(StatusCode::NOT_FOUND, "not_found", "Not found.")
}

async fn api(shared: &Shared, request: Request, tail: &[String]) -> Response {
    if let Some(sessions) = &shared.sessions {
        let cookie = request
            .headers()
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok());
        if !sessions.recognises(cookie) {
            return json_error(
                StatusCode::UNAUTHORIZED,
                "ui_token_required",
                "Enter the access token this service printed when it started.",
            );
        }
    }
    let (parts, body) = request.into_parts();
    if !matches!(
        parts.method,
        Method::GET | Method::HEAD | Method::POST | Method::DELETE
    ) {
        return json_error(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "The admin API is not asked this way.",
        );
    }
    let Ok(bytes) = axum::body::to_bytes(body, forward::MAX_REQUEST_BYTES).await else {
        return json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_too_large",
            "The request body is too large.",
        );
    };
    shared
        .upstream
        .forward(
            &parts.method,
            tail,
            parts.uri.query(),
            &parts.headers,
            bytes,
        )
        .await
}

/// `POST /ui/session` with `{"token": "..."}`: the token printed at start, exchanged for a session cookie.
async fn exchange(shared: &Shared, request: Request) -> Response {
    let Some(sessions) = &shared.sessions else {
        return not_found();
    };
    if *request.method() != Method::POST {
        return json_error(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "Send the token with POST.",
        );
    }
    let Ok(bytes) = axum::body::to_bytes(request.into_body(), 1024).await else {
        return json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_too_large",
            "The request body is too large.",
        );
    };
    let offered = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v["token"].as_str().map(str::to_string));
    if !offered.is_some_and(|token| sessions.token_matches(&token)) {
        return json_error(
            StatusCode::UNAUTHORIZED,
            "ui_token_invalid",
            "That is not the access token.",
        );
    }
    let Ok(id) = new_token() else {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "ui_session_failed",
            "A session could not be made.",
        );
    };
    sessions.remember(id.clone());
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::NO_CONTENT;
    if let Ok(cookie) = HeaderValue::from_str(&format!(
        "{}={id}; HttpOnly; SameSite=Strict; Path=/",
        guard::COOKIE
    )) {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    response
}

/// A file of the GUI, or its entry page for a path that is a client-side route.
fn asset(method: &Method, segments: &[String]) -> Response {
    if !matches!(*method, Method::GET | Method::HEAD) {
        return json_error(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "Only GET and HEAD are answered here.",
        );
    }
    let name = segments.join("/");
    let found = crate::ui_assets::lookup(&name);
    // A path with a file extension that is not a file is a missing file; any other unknown path is a route of the
    // single-page app, and the page loads it.
    let has_extension = segments.last().is_some_and(|last| last.contains('.'));
    let asset = match found {
        Some(asset) => asset,
        None if has_extension => return not_found(),
        None => crate::ui_assets::page(),
    };
    let mut response = Response::new(Body::from(asset.body));
    let cache = if asset.immutable {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(asset.content_type),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}
