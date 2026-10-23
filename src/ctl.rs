//! `humpyard ctl`: a client for a running gateway, local or remote (spec: `cli`).
//!
//! Every subcommand is one request to an endpoint the gateway already serves, made with the caller's own key, so
//! a key sees exactly what `curl` would show it. All of them only read, except `overrides`, which can also add
//! and remove runtime overrides when the key may (see the `overrides` module). The key is read only from an environment variable, is never
//! printed, and is never sent where it could be read in transit (plain `http://` only to this machine) or
//! carried elsewhere (redirects are not followed). Only the `ctl` command (`main`) may use this module.

use std::fmt::Write as _;
use std::io::Write;
use std::net::IpAddr;
use std::time::Duration;

use reqwest::{StatusCode, Url};
use serde_json::{Value, json};

mod overrides;

pub use overrides::OverridesCommand;

const TIMEOUT: Duration = Duration::from_secs(30);

/// Why a command did not succeed; the variant decides the exit code.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The gateway answered 401, 403, 404 or 429 (exit 1).
    Refused(String),
    /// The gateway could not be used: unreachable, not understood, a bad address or key setup (exit 2).
    Unusable(String),
}

impl Failure {
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        match self {
            Failure::Refused(_) => 1,
            Failure::Unusable(_) => 2,
        }
    }

    #[must_use]
    pub fn message(&self) -> &str {
        match self {
            Failure::Refused(m) | Failure::Unusable(m) => m,
        }
    }
}

/// What the user typed, before it is checked.
pub struct Settings {
    pub url: Option<String>,
    pub key_env: String,
    pub allow_insecure_http: bool,
    pub json: bool,
}

pub enum Command {
    Status,
    Models,
    Health,
    Stats {
        window: Option<String>,
        group_by: Option<String>,
        route: Option<String>,
        target: Option<String>,
    },
    Decisions {
        limit: Option<u32>,
        route: Option<String>,
        rule: Option<String>,
        outcome: Option<String>,
        request_id: Option<String>,
    },
    Quota,
    Config {
        fingerprint: bool,
    },
    Explain {
        model: Option<String>,
        prompt_tokens: Option<u64>,
        tools: bool,
        images: bool,
        subagent: bool,
        body_file: Option<String>,
    },
    Whoami,
    /// Runtime overrides: list, add and remove.
    Overrides(OverridesCommand),
}

/// A checked address and key.
pub struct Gateway {
    base: Url,
    key: String,
    client: reqwest::Client,
}

fn is_local(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Checks the address and reads the key from the environment.
pub fn connect(
    settings: &Settings,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Gateway, Failure> {
    let unusable = |m: String| Failure::Unusable(m);
    let address = settings
        .url
        .clone()
        .or_else(|| env("HUMPYARD_URL"))
        .filter(|u| !u.is_empty())
        .ok_or_else(|| unusable("name the gateway with --url or HUMPYARD_URL".into()))?;
    let base = Url::parse(&address).map_err(|e| {
        unusable(format!(
            "`{address}` is not an address like https://host: {e}"
        ))
    })?;
    match base.scheme() {
        "https" => {}
        "http" if is_local(&base) || settings.allow_insecure_http => {}
        "http" => {
            return Err(unusable(format!(
                "{address} is plain http to another machine, which would send the key unprotected; use https, \
                 a port-forward to 127.0.0.1, or --allow-insecure-http"
            )));
        }
        other => {
            return Err(unusable(format!(
                "the address scheme must be http or https, not {other}"
            )));
        }
    }
    let key = env(&settings.key_env)
        .filter(|k| !k.is_empty())
        .ok_or_else(|| {
            unusable(format!(
                "the key is read from ${}, which is not set",
                settings.key_env
            ))
        })?;
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| unusable(e.to_string()))?;
    Ok(Gateway { base, key, client })
}

fn gateway_message(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    v["error"]["message"].as_str().map(str::to_string)
}

impl Gateway {
    async fn send(&self, request: reqwest::RequestBuilder) -> Result<String, Failure> {
        let response = request.bearer_auth(&self.key).send().await.map_err(|e| {
            Failure::Unusable(if e.is_timeout() {
                "the gateway did not answer within 30 s".into()
            } else {
                format!("cannot reach {}", self.base)
            })
        })?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|_| Failure::Unusable("the answer could not be read".into()))?;
        if status.is_redirection() {
            return Err(Failure::Unusable(format!(
                "the gateway answered {status} with a redirect; it was not followed, so the key stays here"
            )));
        }
        if status.is_success() {
            return Ok(body);
        }
        let text = gateway_message(&body).unwrap_or_default();
        let message = format!("{status} {text}").trim().to_string();
        if matches!(
            status,
            StatusCode::UNAUTHORIZED
                | StatusCode::FORBIDDEN
                | StatusCode::NOT_FOUND
                | StatusCode::TOO_MANY_REQUESTS
        ) {
            Err(Failure::Refused(message))
        } else {
            Err(Failure::Unusable(format!("the gateway answered {message}")))
        }
    }

    fn url(&self, path: &str) -> Result<Url, Failure> {
        self.base
            .join(path.trim_start_matches('/'))
            .map_err(|e| Failure::Unusable(e.to_string()))
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<String, Failure> {
        let mut url = self.url(path)?;
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        self.send(self.client.get(url)).await
    }

    async fn post(&self, path: &str, body: &Value) -> Result<String, Failure> {
        let url = self.url(path)?;
        self.send(self.client.post(url).json(body)).await
    }

    async fn delete(&self, path: &str) -> Result<String, Failure> {
        let url = self.url(path)?;
        self.send(self.client.delete(url)).await
    }
}

fn parse(body: &str) -> Result<Value, Failure> {
    serde_json::from_str(body)
        .map_err(|_| Failure::Unusable("the gateway's answer was not JSON".into()))
}

// ---- rendering: pure functions over the response, so they are tested without a gateway ----

fn scalar(v: &Value) -> String {
    match v {
        Value::Null => "-".into(),
        Value::String(s) => s.clone(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() => {
                let text = format!("{f:.4}");
                text.trim_end_matches('0').trim_end_matches('.').to_string()
            }
            _ => n.to_string(),
        },
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn at<'a>(v: &'a Value, path: &str) -> &'a Value {
    path.split('.').fold(v, |cur, part| &cur[part])
}

/// Indented `key: value` lines for any JSON value, so a field the gateway adds later still shows.
#[must_use]
pub fn tree(v: &Value, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut out = String::new();
    let Value::Object(map) = v else {
        let _ = writeln!(out, "{pad}{}", scalar(v));
        return out;
    };
    for (k, val) in map {
        match val {
            Value::Object(_) => {
                let _ = write!(out, "{pad}{k}:\n{}", tree(val, indent + 2));
            }
            Value::Array(items) if items.iter().any(|i| i.is_object() || i.is_array()) => {
                let _ = writeln!(out, "{pad}{k}:");
                for item in items {
                    let _ = writeln!(out, "{pad}  - {}", compact(item));
                }
            }
            Value::Array(items) => {
                let list: Vec<String> = items.iter().map(scalar).collect();
                let shown = if list.is_empty() {
                    "-".to_string()
                } else {
                    list.join(", ")
                };
                let _ = writeln!(out, "{pad}{k}: {shown}");
            }
            other => {
                let _ = writeln!(out, "{pad}{k}: {}", scalar(other));
            }
        }
    }
    out
}

fn compact(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// A left-aligned table of the given columns (header, dotted path) over `rows`.
#[must_use]
pub fn table(rows: &[Value], columns: &[(&str, &str)]) -> String {
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| columns.iter().map(|(_, p)| scalar(at(r, p))).collect())
        .collect();
    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(i, (h, _))| {
            cells
                .iter()
                .map(|c| c[i].chars().count())
                .chain([h.chars().count()])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |row: Vec<&str>| {
        let parts: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(c, w)| format!("{c:<w$}"))
            .collect();
        format!("{}\n", parts.join("  ").trim_end())
    };
    let mut out = line(columns.iter().map(|(h, _)| *h).collect());
    for row in &cells {
        out.push_str(&line(row.iter().map(String::as_str).collect()));
    }
    out
}

#[must_use]
pub fn render_models(v: &Value) -> String {
    let ids: Vec<&str> = v["data"]
        .as_array()
        .map(|a| a.iter().filter_map(|m| m["id"].as_str()).collect())
        .unwrap_or_default();
    if ids.is_empty() {
        "no routes or targets are available to this key\n".into()
    } else {
        format!("{}\n", ids.join("\n"))
    }
}

#[must_use]
pub fn render_health(v: &Value) -> String {
    let mut out = format!(
        "config {}  (loaded at {} ms)\nreload: ok={}, attempts={}\n\n",
        scalar(&v["config"]["fingerprint"]),
        scalar(&v["config"]["loaded_at_ms"]),
        scalar(&v["reload"]["ok"]),
        scalar(&v["reload"]["attempts"])
    );
    let mut rows = Vec::new();
    for t in v["targets"].as_array().cloned().unwrap_or_default() {
        for e in t["endpoints"].as_array().cloned().unwrap_or_default() {
            rows.push(json!({
                "target": t["target"], "latency": t["current_latency_ms"],
                "endpoint": format!("{}/{}", scalar(&e["provider"]), scalar(&e["model"])),
                "state": e["state"], "failures": e["consecutive_failures"],
                "cooldown": e["cooldown_remaining_ms"],
            }));
        }
    }
    if rows.is_empty() {
        out.push_str("no endpoints reported\n");
    } else {
        out.push_str(&table(
            &rows,
            &[
                ("TARGET", "target"),
                ("ENDPOINT", "endpoint"),
                ("STATE", "state"),
                ("FAILS", "failures"),
                ("COOLDOWN ms", "cooldown"),
                ("LATENCY ms", "latency"),
            ],
        ));
    }
    out
}

#[must_use]
pub fn render_stats(v: &Value) -> String {
    let rows = v["data"].as_array().cloned().unwrap_or_default();
    let mut out = format!(
        "grouped by {} over the last {} s{}\n\n",
        scalar(&v["group_by"]),
        scalar(&v["window"]["seconds"]),
        if v["truncated"].as_bool().unwrap_or(false) {
            " (truncated)"
        } else {
            ""
        }
    );
    if rows.is_empty() {
        out.push_str("no requests in this window\n");
        return out;
    }
    out.push_str(&table(
        &rows,
        &[
            ("GROUP", "group"),
            ("REQUESTS", "requests"),
            ("FAIL RATE", "failure_rate"),
            ("FALLBACK RATE", "fallback_rate"),
            ("P50 ms", "latency_ms.p50"),
            ("P90 ms", "latency_ms.p90"),
            ("P99 ms", "latency_ms.p99"),
            ("AVG COST USD", "avg_cost_usd"),
        ],
    ));
    out
}

#[must_use]
pub fn render_decisions(v: &Value) -> String {
    let rows = v["data"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        return "no decisions match\n".into();
    }
    table(
        &rows,
        &[
            ("REQUEST", "request_id"),
            ("MODEL", "requested_model"),
            ("ROUTE", "route"),
            ("RULE", "rule"),
            ("SOURCE", "source"),
            ("TS ms", "ts_ms"),
        ],
    )
}

#[must_use]
pub fn render_config(v: &Value) -> String {
    let mut out = format!(
        "fingerprint {}  (loaded at {} ms)\n\n",
        scalar(&v["config"]["fingerprint"]),
        scalar(&v["config"]["loaded_at_ms"])
    );
    out.push_str("settings:\n");
    out.push_str(&tree(&v["settings"], 2));
    let rows = |section: &str, f: &dyn Fn(&str, &Value) -> Value| -> Vec<Value> {
        v[section]
            .as_object()
            .map(|m| m.iter().map(|(k, val)| f(k, val)).collect())
            .unwrap_or_default()
    };
    let providers = rows(
        "providers",
        &|name, p| json!({"name": name, "base_url": p["base_url"], "timeout": p["timeout_secs"], "retries": p["max_retries"]}),
    );
    out.push_str("\nproviders:\n");
    out.push_str(&table(
        &providers,
        &[
            ("NAME", "name"),
            ("BASE URL", "base_url"),
            ("TIMEOUT s", "timeout"),
            ("RETRIES", "retries"),
        ],
    ));
    let targets = rows("targets", &|name, t| {
        let first = &t["endpoints"][0];
        json!({"name": name, "source": t["source"], "class": t["class"], "quality": t["quality"],
               "endpoint": format!("{}/{}", scalar(&first["provider"]), scalar(&first["model"])),
               "endpoints": t["endpoints"].as_array().map_or(0, Vec::len)})
    });
    out.push_str("\ntargets:\n");
    out.push_str(&table(
        &targets,
        &[
            ("NAME", "name"),
            ("SOURCE", "source"),
            ("CLASS", "class"),
            ("QUALITY", "quality"),
            ("FIRST ENDPOINT", "endpoint"),
            ("#", "endpoints"),
        ],
    ));
    let routes = rows("routes", &|name, r| {
        let members = r["resolved"]["members"].as_array().map(Vec::len);
        json!({"name": name, "type": r["type"], "members": members, "classes": r["allowed_classes"], "tail": r["then"]["targets"]})
    });
    out.push_str("\nroutes:\n");
    out.push_str(&table(
        &routes,
        &[
            ("NAME", "name"),
            ("TYPE", "type"),
            ("POOL MEMBERS", "members"),
            ("CLASSES", "classes"),
            ("TAIL", "tail"),
        ],
    ));
    let keys = rows(
        "keys",
        &|id, k| json!({"id": id, "routes": k["allowed_routes"], "read_config": k["read_config"]}),
    );
    if !keys.is_empty() {
        out.push_str("\nkeys:\n");
        out.push_str(&table(
            &keys,
            &[
                ("ID", "id"),
                ("ROUTES", "routes"),
                ("READ CONFIG", "read_config"),
            ],
        ));
    }
    out
}

/// What `explain` needs: the body the user gave, or one built from the flags.
fn explain_body(command: &Command) -> Result<Value, Failure> {
    let Command::Explain {
        model,
        prompt_tokens,
        tools,
        images,
        subagent,
        body_file,
    } = command
    else {
        return Err(Failure::Unusable("not an explain command".into()));
    };
    if let Some(path) = body_file {
        let text = std::fs::read_to_string(path)
            .map_err(|e| Failure::Unusable(format!("cannot read {path}: {e}")))?;
        return serde_json::from_str(&text)
            .map_err(|e| Failure::Unusable(format!("{path} is not JSON: {e}")));
    }
    let model = model
        .clone()
        .ok_or_else(|| Failure::Unusable("explain needs --model (or --body FILE)".into()))?;
    let mut body = json!({"model": model, "tools": tools, "images": images, "subagent": subagent});
    if let Some(n) = prompt_tokens {
        body["prompt_tokens"] = json!(n);
    }
    Ok(body)
}

fn query(pairs: Vec<(&'static str, Option<String>)>) -> Vec<(&'static str, String)> {
    pairs
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect()
}

/// Runs one command and writes its result to `out`.
pub async fn run(
    gateway: &Gateway,
    command: Command,
    json_output: bool,
    out: &mut dyn Write,
) -> Result<(), Failure> {
    let mut emit = |text: &str| {
        let _ = out.write_all(text.as_bytes());
    };
    let (path, params, post_body): (&str, Vec<(&str, String)>, Option<Value>) = match &command {
        Command::Status => return status(gateway, json_output, out).await,
        Command::Overrides(overrides) => {
            return overrides::run(gateway, overrides, json_output, out).await;
        }
        Command::Models => ("/v1/models", vec![], None),
        Command::Health => ("/v1/health", vec![], None),
        Command::Quota => ("/v1/quota", vec![], None),
        Command::Whoami => ("/v1/key/info", vec![], None),
        Command::Config { .. } => ("/v1/config", vec![], None),
        Command::Stats {
            window,
            group_by,
            route,
            target,
        } => (
            "/v1/stats",
            query(vec![
                ("window", window.clone()),
                ("group_by", group_by.clone()),
                ("route", route.clone()),
                ("target", target.clone()),
            ]),
            None,
        ),
        Command::Decisions {
            limit,
            route,
            rule,
            outcome,
            request_id,
        } => (
            "/v1/decisions",
            query(vec![
                ("limit", limit.map(|n| n.to_string())),
                ("route", route.clone()),
                ("rule", rule.clone()),
                ("outcome", outcome.clone()),
                ("request_id", request_id.clone()),
            ]),
            None,
        ),
        Command::Explain { .. } => ("/v1/route/explain", vec![], Some(explain_body(&command)?)),
    };
    let body = match &post_body {
        Some(b) => gateway.post(path, b).await?,
        None => gateway.get(path, &params).await?,
    };
    if json_output {
        emit(&format!("{}\n", body.trim_end()));
        return Ok(());
    }
    let v = parse(&body)?;
    if matches!(command, Command::Config { fingerprint: true }) {
        emit(&format!("{}\n", scalar(&v["config"]["fingerprint"])));
        return Ok(());
    }
    let text = match command {
        Command::Models => render_models(&v),
        Command::Health => render_health(&v),
        Command::Stats { .. } => render_stats(&v),
        Command::Decisions { .. } => render_decisions(&v),
        Command::Config { .. } => render_config(&v),
        _ => tree(&v, 0),
    };
    emit(&text);
    Ok(())
}

async fn status(gateway: &Gateway, json_output: bool, out: &mut dyn Write) -> Result<(), Failure> {
    // Reachability is the one thing `status` must establish; the rest degrades to a note.
    gateway.get("/healthz", &[]).await?;
    let health = gateway.get("/v1/health", &[]).await;
    let key = gateway.get("/v1/key/info", &[]).await;
    if json_output {
        let part = |r: &Result<String, Failure>| match r {
            Ok(b) => serde_json::from_str(b).unwrap_or(Value::Null),
            Err(f) => json!({"error": f.message()}),
        };
        let doc = json!({"reachable": true, "health": part(&health), "key": part(&key)});
        let _ = writeln!(out, "{doc}");
        return Ok(());
    }
    let _ = writeln!(out, "gateway: reachable at {}", gateway.base);
    match health.and_then(|b| parse(&b)) {
        Ok(v) => {
            let _ = writeln!(
                out,
                "config: {}  reload ok: {}",
                scalar(&v["config"]["fingerprint"]),
                scalar(&v["reload"]["ok"])
            );
            let mut skipped = 0;
            for t in v["targets"].as_array().cloned().unwrap_or_default() {
                for e in t["endpoints"].as_array().cloned().unwrap_or_default() {
                    if e["state"]
                        .as_str()
                        .is_some_and(|s| s != "closed" && s != "healthy")
                    {
                        skipped += 1;
                        let _ = writeln!(
                            out,
                            "  skipped: {}/{} on {} ({})",
                            scalar(&e["provider"]),
                            scalar(&e["model"]),
                            scalar(&t["target"]),
                            scalar(&e["state"])
                        );
                    }
                }
            }
            let _ = writeln!(out, "endpoints being skipped: {skipped}");
        }
        Err(f) => {
            let _ = writeln!(out, "health: unavailable ({})", f.message());
        }
    }
    match key.and_then(|b| parse(&b)) {
        Ok(v) => {
            let _ = writeln!(
                out,
                "key: {}  budget {}  spent today {} USD, {} tokens",
                scalar(&v["id"]),
                scalar(&v["state"]),
                scalar(&v["spend"]["daily_usd"]),
                scalar(&v["spend"]["daily_tokens"])
            );
        }
        Err(f) => {
            let _ = writeln!(out, "key: unavailable ({})", f.message());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(url: Option<&str>) -> Settings {
        Settings {
            url: url.map(String::from),
            key_env: "K".into(),
            allow_insecure_http: false,
            json: false,
        }
    }

    fn env(name: &str) -> Option<String> {
        (name == "K").then(|| "secret-value".to_string())
    }

    #[test]
    fn plain_http_is_for_this_machine_only() {
        assert!(connect(&settings(Some("http://127.0.0.1:8080")), &env).is_ok());
        assert!(connect(&settings(Some("http://localhost:8080")), &env).is_ok());
        assert!(connect(&settings(Some("http://[::1]:8080")), &env).is_ok());
        assert!(connect(&settings(Some("https://gateway.example")), &env).is_ok());
        let refused = connect(&settings(Some("http://gateway.example")), &env);
        assert!(matches!(refused, Err(Failure::Unusable(m)) if m.contains("plain http")));
        let mut allowed = settings(Some("http://gateway.example"));
        allowed.allow_insecure_http = true;
        assert!(connect(&allowed, &env).is_ok());
    }

    #[test]
    fn an_unset_key_variable_is_named_and_the_address_may_come_from_the_environment() {
        let none = |_: &str| None;
        let err = connect(&settings(Some("https://g.example")), &none).err();
        assert!(matches!(err, Some(Failure::Unusable(m)) if m.contains("$K")));
        let err = connect(&settings(None), &env).err();
        assert!(matches!(err, Some(Failure::Unusable(m)) if m.contains("HUMPYARD_URL")));
        let from_env = |n: &str| match n {
            "HUMPYARD_URL" => Some("https://g.example".to_string()),
            _ => env(n),
        };
        assert!(connect(&settings(None), &from_env).is_ok());
    }

    #[test]
    fn exit_codes_follow_the_failure() {
        assert_eq!(Failure::Refused("x".into()).exit_code(), 1);
        assert_eq!(Failure::Unusable("x".into()).exit_code(), 2);
    }

    #[test]
    fn the_table_aligns_columns_and_shows_missing_fields_as_a_dash() {
        let rows = vec![json!({"a": "long-value", "b": 1.5}), json!({"a": "x"})];
        let text = table(&rows, &[("A", "a"), ("B", "b")]);
        assert_eq!(text, "A           B\nlong-value  1.5\nx           -\n");
    }

    #[test]
    fn the_tree_shows_nested_fields_and_lists() {
        let text = tree(
            &json!({"id": "alice", "limits": {"daily_usd": 5.0}, "tags": ["a", "b"]}),
            0,
        );
        assert!(text.contains("id: alice\n"));
        assert!(text.contains("limits:\n  daily_usd: 5\n"));
        assert!(text.contains("tags: a, b\n"));
    }
}
