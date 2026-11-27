//! `humpyard init`: a guided first config.
//!
//! The user names providers; each key is read only from the environment variable the config will use, and its
//! value is never printed, logged or written. For every provider whose key is set, the model list is fetched and
//! each chosen model must answer one small request before it is written, so a retired model can never reach the
//! file (the same "the provider must say it exists" rule as discovery, ADR 0022). The file is loaded with the
//! normal parser before it is written, never overwrites an existing file, and listens on loopback unless the
//! user asks otherwise, in which case a virtual key with a token budget is created and shown once. Only the
//! `init` command (`main`) may use this module, so `serve` can never fetch it.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{BufRead, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::config::Config;
use crate::discover::fetch_provider_models;

const CONFIRM_TIMEOUT: Duration = Duration::from_secs(45);
const DEFAULT_LISTEN: &str = "127.0.0.1:8080";
/// What a key made for a non-loopback listener may use in a day, so an exposed gateway is bounded from the start.
const SHARED_DAILY_TOKENS: u64 = 2_000_000;

/// A provider the command knows how to configure. The model lists are preferences, checked live, so a
/// retired id degrades to a message and never to a broken file.
pub struct Preset {
    pub name: &'static str,
    pub base_url: &'static str,
    pub key_env: &'static str,
    pub fast: &'static [&'static str],
    pub deep: &'static [&'static str],
}

pub const PRESETS: [Preset; 3] = [
    Preset {
        name: "groq",
        base_url: "https://api.groq.com/openai/v1",
        key_env: "GROQ_API_KEY",
        fast: &["openai/gpt-oss-20b"],
        deep: &["openai/gpt-oss-120b"],
    },
    Preset {
        name: "openrouter",
        base_url: "https://openrouter.ai/api/v1",
        key_env: "OPENROUTER_API_KEY",
        fast: &[
            "openai/gpt-oss-20b:free",
            "meta-llama/llama-3.3-70b-instruct:free",
        ],
        deep: &["openai/gpt-oss-120b:free"],
    },
    Preset {
        name: "nvidia",
        base_url: "https://integrate.api.nvidia.com/v1",
        key_env: "NVIDIA_API_KEY",
        fast: &["openai/gpt-oss-20b"],
        deep: &["openai/gpt-oss-120b", "nvidia/nemotron-3-super-120b-a12b"],
    },
];

#[must_use]
pub fn preset(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}

/// One provider as it will be written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub name: String,
    pub base_url: String,
    pub key_env: String,
    pub fast: Option<String>,
    pub deep: Option<String>,
}

/// What the user asked for.
pub struct InitOptions {
    /// Preset names.
    pub providers: Vec<String>,
    /// `NAME,BASE_URL,KEY_ENV,MODEL` for an OpenAI-compatible endpoint that is not a preset.
    pub custom: Vec<String>,
    pub out: PathBuf,
    pub listen: String,
    /// Skip the in-process test request.
    pub no_proof: bool,
}

/// The first preferred model the provider lists.
#[must_use]
pub fn first_listed(listed: &BTreeSet<String>, preferred: &[&str]) -> Option<String> {
    preferred
        .iter()
        .find(|m| listed.contains(**m))
        .map(|m| (*m).to_string())
}

/// Whether a `listen` address stays on this machine.
pub fn is_loopback(listen: &str) -> Result<bool, String> {
    listen
        .parse::<SocketAddr>()
        .map(|a| a.ip().is_loopback())
        .map_err(|e| format!("`--listen {listen}` is not an address like 127.0.0.1:8080: {e}"))
}

fn toml_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The configuration text. `key` is the hash of a virtual key for a gateway that is not loopback-only.
#[must_use]
pub fn render(picks: &[Pick], listen: &str, key_hash: Option<&str>) -> String {
    let mut out = String::from(
        "# Written by `humpyard init`. Clients ask for the model `fast` or `deep`.\n\
         # Keys are read from the environment variables named below; none is stored here.\n",
    );
    let _ = writeln!(out, "listen = {}\n", toml_string(listen));
    if key_hash.is_some() {
        out.push_str("[ledger]\npath = \"humpyard.db\"\n\n");
    }
    for p in picks {
        let _ = writeln!(
            out,
            "[providers.{}]\nbase_url = {}\napi_key_env = {}\n",
            p.name,
            toml_string(&p.base_url),
            toml_string(&p.key_env)
        );
    }
    for (target, pick_of) in [
        ("fast", (|p: &Pick| p.fast.clone()) as fn(&Pick) -> _),
        ("deep", |p: &Pick| p.deep.clone()),
    ] {
        let endpoints: Vec<String> = picks
            .iter()
            .filter_map(|p| {
                pick_of(p).map(|m| {
                    format!(
                        "{{ provider = {}, model = {} }}",
                        toml_string(&p.name),
                        toml_string(&m)
                    )
                })
            })
            .collect();
        if !endpoints.is_empty() {
            let _ = writeln!(
                out,
                "[targets.{target}]\nendpoints = [{}]\n",
                endpoints.join(", ")
            );
        }
    }
    if let Some(hash) = key_hash {
        let _ = writeln!(
            out,
            "[keys.default]\nsha256 = {}\ndaily_tokens = {SHARED_DAILY_TOKENS}\nover_budget = \"block\"",
            toml_string(hash)
        );
    }
    out
}

/// Parses `NAME,BASE_URL,KEY_ENV,MODEL`.
pub fn parse_custom(text: &str) -> Result<Pick, String> {
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    let [name, base_url, key_env, model] = parts[..] else {
        return Err(format!(
            "`--custom {text}` must be NAME,BASE_URL,KEY_ENV,MODEL"
        ));
    };
    let name_ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !name_ok || base_url.is_empty() || key_env.is_empty() || model.is_empty() {
        return Err(format!(
            "`--custom {text}`: the name may use letters, digits, - and _, and no part may be empty"
        ));
    }
    Ok(Pick {
        name: name.to_string(),
        base_url: base_url.to_string(),
        key_env: key_env.to_string(),
        fast: Some(model.to_string()),
        deep: None,
    })
}

/// Asks which presets to use, from a list, on a terminal.
fn ask_providers(input: &mut dyn BufRead, out: &mut dyn Write) -> Result<Vec<String>, String> {
    let _ = writeln!(out, "Which providers do you have a key for?");
    for (i, p) in PRESETS.iter().enumerate() {
        let _ = writeln!(out, "  {}) {:<11} key in ${}", i + 1, p.name, p.key_env);
    }
    let _ = write!(out, "Numbers or names, separated by commas: ");
    let _ = out.flush();
    let mut line = String::new();
    input
        .read_line(&mut line)
        .map_err(|e| format!("cannot read your answer: {e}"))?;
    let mut chosen = Vec::new();
    for word in line.split(',').map(str::trim).filter(|w| !w.is_empty()) {
        let name = word
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| PRESETS.get(i))
            .map(|p| p.name)
            .or_else(|| preset(word).map(|p| p.name))
            .ok_or_else(|| format!("`{word}` is not one of the listed providers"))?;
        chosen.push(name.to_string());
    }
    Ok(chosen)
}

/// Whether a provider accepts a key: it lists its models with it and, when a model is named, answers one small
/// request for it. The message never contains the key.
pub(crate) async fn check_key(
    base_url: &str,
    key: &str,
    model: Option<&str>,
) -> Result<(), String> {
    fetch_provider_models(base_url, key)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(model) = model {
        confirm_model(base_url, key, model)
            .await
            .map_err(|why| format!("it lists models, but {model} {why}"))?;
    }
    Ok(())
}

/// One small request to the model; `Ok` when the provider answers it.
async fn confirm_model(base_url: &str, key: &str, model: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(CONFIRM_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 64,
        "messages": [{"role": "user", "content": "Reply with the single word: ok"}],
    });
    let response = client
        .post(&url)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                "did not answer within 45 s".to_string()
            } else {
                "could not be reached".to_string()
            }
        })?;
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("answered {status}"))
    }
}

/// The preferred fast and deep models of a preset.
type Prefs = (&'static [&'static str], &'static [&'static str]);

/// A provider to check, with the preferences to choose its models by (none for a custom endpoint).
struct Candidate {
    pick: Pick,
    prefs: Option<Prefs>,
}

/// The providers the user asked for, as candidates.
fn candidates(names: &[String], custom: &[String]) -> Result<Vec<Candidate>, String> {
    let mut out = Vec::new();
    for name in names {
        let p = preset(name).ok_or_else(|| {
            let known: Vec<&str> = PRESETS.iter().map(|p| p.name).collect();
            format!(
                "`{name}` is not a built-in provider ({}); use --custom for another",
                known.join(", ")
            )
        })?;
        out.push(Candidate {
            pick: Pick {
                name: p.name.into(),
                base_url: p.base_url.into(),
                key_env: p.key_env.into(),
                fast: None,
                deep: None,
            },
            prefs: Some((p.fast, p.deep)),
        });
    }
    for text in custom {
        out.push(Candidate {
            pick: parse_custom(text)?,
            prefs: None,
        });
    }
    let mut seen = BTreeSet::new();
    if let Some(dup) = out.iter().find(|c| !seen.insert(c.pick.name.clone())) {
        return Err(format!("provider `{}` is listed twice", dup.pick.name));
    }
    Ok(out)
}

/// The result of checking one provider.
struct Checked {
    pick: Pick,
    /// Whether the key was present and the choices were confirmed live.
    live: bool,
}

async fn check_provider(
    mut pick: Pick,
    prefs: Option<Prefs>,
    key: Option<String>,
    out: &mut dyn Write,
) -> Option<Checked> {
    let Some(key) = key else {
        let _ = writeln!(
            out,
            "  {}: ${} is not set, so its models were not checked; export it before `serve`",
            pick.name, pick.key_env
        );
        if let Some((fast, deep)) = prefs {
            pick.fast = fast.first().map(|m| (*m).to_string());
            pick.deep = deep.first().map(|m| (*m).to_string());
        }
        return Some(Checked { pick, live: false });
    };
    let listed = match fetch_provider_models(&pick.base_url, &key).await {
        Ok(listed) => listed,
        Err(e) => {
            let _ = writeln!(out, "  {}: skipped, {e}", pick.name);
            return None;
        }
    };
    if let Some((fast, deep)) = prefs {
        pick.fast = first_listed(&listed, fast);
        pick.deep = first_listed(&listed, deep);
    } else if pick.fast.as_ref().is_some_and(|m| !listed.contains(m)) {
        let _ = writeln!(
            out,
            "  {}: {} is not in its model list",
            pick.name,
            pick.fast.clone().unwrap_or_default()
        );
        pick.fast = None;
    }
    for (label, slot) in [("fast", &mut pick.fast), ("deep", &mut pick.deep)] {
        if let Some(model) = slot.clone() {
            match confirm_model(&pick.base_url, &key, &model).await {
                Ok(()) => {
                    let _ = writeln!(out, "  {}: {label} = {model} (answered)", pick.name);
                }
                Err(why) => {
                    let _ = writeln!(out, "  {}: {label} {model} left out, {why}", pick.name);
                    *slot = None;
                }
            }
        }
    }
    if pick.fast.is_none() && pick.deep.is_none() {
        let sample: Vec<&str> = listed.iter().map(String::as_str).take(12).collect();
        let _ = writeln!(
            out,
            "  {}: none of the usual models is available; it lists, for example: {}",
            pick.name,
            sample.join(", ")
        );
        return None;
    }
    Some(Checked { pick, live: true })
}

/// Sends one request through the written setup, in process on a loopback port, and says what happened.
async fn prove(
    picks: &[Pick],
    env: &dyn Fn(&str) -> Option<String>,
    out: &mut dyn Write,
) -> Result<(), String> {
    let text = render(picks, "127.0.0.1:0", None);
    let config = Config::from_toml(&text, env).map_err(|e| e.to_string())?;
    let app = crate::server::router(config).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    let started = Instant::now();
    let model = if picks.iter().any(|p| p.fast.is_some()) {
        "fast"
    } else {
        "deep"
    };
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/v1/chat/completions"))
        .json(&serde_json::json!({
            "model": model,
            "max_tokens": 200,
            "messages": [{"role": "user", "content": "In one short sentence, say hello."}],
        }))
        .timeout(Duration::from_secs(90))
        .send()
        .await;
    server.abort();
    let response = response.map_err(|e| format!("the test request failed: {e}"))?;
    let status = response.status();
    let target = response
        .headers()
        .get("x-humpyard-target")
        .and_then(|v| v.to_str().ok())
        .unwrap_or(model)
        .to_string();
    let body: Value = response.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("the test request answered {status}"));
    }
    let reply = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim();
    let _ = writeln!(
        out,
        "\nTest request to `{model}`: served by target `{target}` in {} ms.\nReply: {reply}",
        started.elapsed().as_millis()
    );
    Ok(())
}

fn snippets(listen: &str, file: &std::path::Path, out: &mut dyn Write) {
    let url = format!("http://{listen}");
    let _ = writeln!(
        out,
        "\nRun it:\n  humpyard serve --config {}\n\n\
         Use it:\n  curl {url}/v1/chat/completions -H 'content-type: application/json' \\\n    \
         -d '{{\"model\":\"fast\",\"messages\":[{{\"role\":\"user\",\"content\":\"hi\"}}]}}'\n  \
         Claude Code: ANTHROPIC_BASE_URL={url} (ask for the models `fast` or `deep`)\n  \
         OpenAI clients: base URL {url}/v1",
        file.display()
    );
}

/// Runs the command. `env` reads the environment (injected for tests); `input` is the terminal when the
/// providers are to be asked for. Progress and status messages go to `progress`. If `options.out` is `-`,
/// the rendered configuration is written to `config` instead of creating a file on disk.
pub async fn run(
    options: InitOptions,
    env: &dyn Fn(&str) -> Option<String>,
    input: Option<&mut dyn BufRead>,
    progress: &mut dyn Write,
    config: &mut dyn Write,
) -> Result<(), String> {
    let is_stdout = options.out == std::path::Path::new("-");
    if !is_stdout && options.out.exists() {
        return Err(format!(
            "{} already exists; init never overwrites a file (choose another --out)",
            options.out.display()
        ));
    }
    let exposed = !is_loopback(&options.listen)?;
    let mut names = options.providers.clone();
    if names.is_empty() && options.custom.is_empty() {
        let input = input.ok_or(
            "name the providers with --provider NAME (groq, openrouter, nvidia) or --custom, or run this in a terminal",
        )?;
        names = ask_providers(input, progress)?;
    }
    if names.is_empty() && options.custom.is_empty() {
        return Err("no provider chosen".into());
    }
    let todo = candidates(&names, &options.custom)?;

    let _ = writeln!(progress, "Checking providers:");
    let mut checked = Vec::new();
    for Candidate { pick, prefs } in todo {
        let key = env(&pick.key_env).filter(|k| !k.is_empty());
        if let Some(c) = check_provider(pick, prefs, key, progress).await {
            checked.push(c);
        }
    }
    if checked.is_empty() {
        return Err("no provider is usable; nothing was written".into());
    }
    let picks: Vec<Pick> = checked.iter().map(|c| c.pick.clone()).collect();

    let key_hash = if exposed {
        let (key, hash) =
            crate::auth::generate_key().map_err(|e| format!("cannot gather randomness: {e}"))?;
        let _ = writeln!(
            progress,
            "\n{} is reachable from other machines, so a key with a daily token budget was made.\n\
             Key (shown once; store it securely): {key}",
            options.listen
        );
        Some(hash)
    } else {
        None
    };

    let text = render(&picks, &options.listen, key_hash.as_deref());
    let key_names: BTreeSet<String> = picks.iter().map(|p| p.key_env.clone()).collect();
    let loading_env = |name: &str| {
        if key_names.contains(name) {
            Some("placeholder".to_string())
        } else {
            env(name)
        }
    };
    Config::from_toml(&text, loading_env).map_err(|e| format!("the config would not load: {e}"))?;

    if is_stdout {
        config
            .write_all(text.as_bytes())
            .map_err(|e| format!("cannot write to stdout: {e}"))?;
    } else {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&options.out)
            .map_err(|e| format!("cannot create {}: {e}", options.out.display()))?;
        file.write_all(text.as_bytes())
            .map_err(|e| format!("cannot write {}: {e}", options.out.display()))?;
        let _ = writeln!(progress, "\nWrote {}", options.out.display());
    }

    let live: Vec<Pick> = checked
        .iter()
        .filter(|c| c.live)
        .map(|c| c.pick.clone())
        .collect();
    if options.no_proof || live.is_empty() {
        if live.is_empty() {
            let _ = writeln!(
                progress,
                "No test request was sent because no key was set; set one and run `humpyard check-config`."
            );
        }
    } else if let Err(e) = prove(&live, env, progress).await {
        if is_stdout {
            let _ = writeln!(progress, "\nThe test request failed: {e}");
        } else {
            let _ = writeln!(
                progress,
                "\nThe file was written, but the test request failed: {e}"
            );
        }
    }
    if !is_stdout {
        snippets(&options.listen, &options.out, progress);
    }
    Ok(())
}

#[must_use]
pub fn default_listen() -> &'static str {
    DEFAULT_LISTEN
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick(name: &str, fast: Option<&str>, deep: Option<&str>) -> Pick {
        Pick {
            name: name.into(),
            base_url: format!("https://{name}.example/v1"),
            key_env: format!("{}_API_KEY", name.to_uppercase()),
            fast: fast.map(Into::into),
            deep: deep.map(Into::into),
        }
    }

    #[test]
    fn the_first_listed_preference_wins() {
        let listed: BTreeSet<String> = ["b", "c"].iter().map(|s| (*s).to_string()).collect();
        assert_eq!(first_listed(&listed, &["a", "c", "b"]), Some("c".into()));
        assert_eq!(first_listed(&listed, &["a"]), None);
    }

    #[test]
    fn loopback_detection() {
        assert!(is_loopback("127.0.0.1:8080").unwrap_or(false));
        assert!(is_loopback("[::1]:8080").unwrap_or(false));
        assert!(!is_loopback("0.0.0.0:8080").unwrap_or(true));
        assert!(is_loopback("localhost").is_err());
    }

    #[test]
    fn rendering_loads_and_names_only_variables() {
        let picks = vec![
            pick("alpha", Some("m1"), Some("m2")),
            pick("beta", Some("m3"), None),
        ];
        let text = render(&picks, "127.0.0.1:8080", None);
        assert!(text.contains("api_key_env = \"ALPHA_API_KEY\""));
        assert!(text.contains("[targets.fast]"));
        assert!(text.contains("[targets.deep]"));
        assert!(!text.contains("[keys."));
        let config = Config::from_toml(&text, |_| Some("placeholder".into()));
        assert!(config.is_ok(), "{config:?}");
    }

    #[test]
    fn no_deep_target_when_nothing_is_deep() {
        let text = render(&[pick("alpha", Some("m1"), None)], "127.0.0.1:8080", None);
        assert!(!text.contains("[targets.deep]"));
    }

    #[test]
    fn a_shared_listener_gets_a_budgeted_key_and_a_ledger() {
        let text = render(
            &[pick("alpha", Some("m1"), None)],
            "0.0.0.0:8080",
            Some("sha256:00"),
        );
        assert!(text.contains("[ledger]"));
        assert!(text.contains("daily_tokens = 2000000"));
        assert!(text.contains("over_budget = \"block\""));
    }

    #[test]
    fn custom_endpoints_are_parsed_strictly() {
        let p = parse_custom("local,http://127.0.0.1:11434/v1,LOCAL_KEY,llama3");
        assert_eq!(
            p,
            Ok(Pick {
                name: "local".into(),
                base_url: "http://127.0.0.1:11434/v1".into(),
                key_env: "LOCAL_KEY".into(),
                fast: Some("llama3".into()),
                deep: None,
            })
        );
        assert!(parse_custom("only,three,parts").is_err());
        assert!(parse_custom("bad name,u,K,m").is_err());
    }
}
