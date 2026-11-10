//! Forwarding `/admin/v1/*` to the gateway (ADR 0032).
//!
//! Only a method, a path and a body the admin API understands go out, with a short list of request headers; the
//! admin credential is either the browser's own or, with `--credential-env`, this command's, put in its place.
//! Redirects are not followed, time and size are bounded, and nothing the credential could ride back in on is
//! passed to the browser: an answer that contains it is replaced.

use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::Response;
use bytes::Bytes;
use futures::StreamExt;
use reqwest::Url;

use super::{Secret, json_error, path};

/// How long one forwarded request may take, from sending it to the last byte of the answer.
pub(super) const TIMEOUT: Duration = Duration::from_secs(30);

/// The most a forwarded answer may hold. The admin API caps its own windows and result sizes; this is the
/// backstop for a gateway that does not.
pub(super) const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// The most a forwarded request body may hold (a configuration patch is at most 64 KiB).
pub(super) const MAX_REQUEST_BYTES: usize = 256 * 1024;

/// Request headers that are passed on. Everything else, cookies included, is dropped.
const REQUEST_HEADERS: [HeaderName; 3] =
    [header::ACCEPT, header::CONTENT_TYPE, header::IF_NONE_MATCH];

/// Answer headers that are passed back. Hop-by-hop headers, cookies and the gateway's own security headers are not:
/// this command sets its own.
const RESPONSE_HEADERS: [HeaderName; 6] = [
    header::CONTENT_TYPE,
    header::CACHE_CONTROL,
    header::WWW_AUTHENTICATE,
    header::RETRY_AFTER,
    header::ETAG,
    header::LOCATION,
];

pub(super) struct Upstream {
    base: Url,
    client: reqwest::Client,
    credential: Option<Secret>,
}

impl Upstream {
    pub(super) fn new(
        base: Url,
        credential: Option<Secret>,
        timeout: Duration,
    ) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            base,
            client,
            credential,
        })
    }

    /// Sends one request to `/admin/v1/<tail>` and returns the gateway's answer, or the reason there is none.
    pub(super) async fn forward(
        &self,
        method: &Method,
        tail: &[String],
        query: Option<&str>,
        incoming: &HeaderMap,
        body: Bytes,
    ) -> Response {
        let mut url = self.base.clone();
        let mut route = String::from("/admin/v1");
        for segment in tail {
            route.push('/');
            route.push_str(&path::encode(segment));
        }
        url.set_path(&route);
        url.set_query(query);

        let mut request = self.client.request(method.clone(), url).body(body);
        for name in &REQUEST_HEADERS {
            if let Some(value) = incoming.get(name) {
                request = request.header(name, value);
            }
        }
        // The credential this command holds replaces whatever the browser sent; without one the browser's is used.
        match &self.credential {
            Some(secret) => request = request.bearer_auth(secret.expose()),
            None => {
                if let Some(value) = incoming.get(header::AUTHORIZATION) {
                    request = request.header(header::AUTHORIZATION, value);
                }
            }
        }

        let answer = match request.send().await {
            Ok(answer) => answer,
            Err(e) if e.is_timeout() => {
                return json_error(
                    StatusCode::GATEWAY_TIMEOUT,
                    "ui_upstream_timeout",
                    "The gateway took too long to answer.",
                );
            }
            Err(_) => {
                return json_error(
                    StatusCode::BAD_GATEWAY,
                    "ui_upstream_unreachable",
                    "The gateway could not be reached.",
                );
            }
        };
        self.relay(answer).await
    }

    async fn relay(&self, answer: reqwest::Response) -> Response {
        let too_big = || {
            json_error(
                StatusCode::BAD_GATEWAY,
                "ui_upstream_too_large",
                "The gateway's answer is too large.",
            )
        };
        if answer
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
        {
            return too_big();
        }
        let status = answer.status();
        let headers = answer.headers().clone();
        let mut received = Vec::new();
        let mut stream = answer.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let Ok(chunk) = chunk else {
                return json_error(
                    StatusCode::BAD_GATEWAY,
                    "ui_upstream_unreachable",
                    "The gateway's answer was cut off.",
                );
            };
            if received.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return too_big();
            }
            received.extend_from_slice(&chunk);
        }

        let mut out = Response::new(Body::from(received.clone()));
        *out.status_mut() = status;
        for name in &RESPONSE_HEADERS {
            if let Some(value) = headers.get(name) {
                out.headers_mut().insert(name.clone(), value.clone());
            }
        }
        if self.leaks(&received, out.headers()) {
            return json_error(
                StatusCode::BAD_GATEWAY,
                "ui_upstream_refused",
                "The gateway's answer was not passed on.",
            );
        }
        out
    }

    /// Whether an answer holds the credential this command injects: it must never reach the browser, whatever
    /// the gateway echoes.
    fn leaks(&self, body: &[u8], headers: &HeaderMap<HeaderValue>) -> bool {
        let Some(secret) = &self.credential else {
            return false;
        };
        let needle = secret.expose().as_bytes();
        let in_body =
            !needle.is_empty() && body.windows(needle.len()).any(|window| window == needle);
        let in_headers = headers.values().any(|v| {
            !needle.is_empty()
                && v.as_bytes()
                    .windows(needle.len())
                    .any(|window| window == needle)
        });
        in_body || in_headers
    }
}
