use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

use humpyard::config::Config;
use humpyard::discover;
use humpyard::migrate;
use humpyard::prices;
use humpyard::server;

#[derive(Parser)]
#[command(version, about = "Unified LLM gateway")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the gateway.
    Serve {
        #[arg(short, long)]
        config: PathBuf,
    },
    /// Validate a config file without starting the server.
    CheckConfig { path: PathBuf },
    /// Write a first config interactively or from flags: checks each provider's models live, writes a
    /// loopback-only `config.toml`, and sends one test request. Reads keys only from the environment
    /// variables the config names; never prints, stores or asks for a key value, and never overwrites a file.
    Init(InitArgs),
    /// Look at a running gateway, local or remote: status, models, health, stats, decisions, quota, explain,
    /// whoami. Read-only; uses your key from an environment variable and never prints it.
    Ctl(CtlArgs),
    /// Create a virtual key: prints the key once and the config block holding its hash.
    Keygen { id: String },
    /// Propose prices from a public source, or check a config's prices for drift. Reads a source
    /// over the network and never changes the config or a running gateway.
    Prices {
        #[command(subcommand)]
        command: PricesCommand,
    },
    #[allow(clippy::doc_markdown)] // "ModelRelay" is a product name (this is also the --help text)
    /// Print a humpyard config fragment for a ModelRelay setup: providers, one target per usable model
    /// (its tags as strengths, its context window), and `auto-fastest` / `tag:<strength>` pool routes.
    /// Reads ModelRelay's admin API (or two saved responses); never copies a key and never edits a file.
    MigrateModelrelay(MigrateArgs),
    /// Find the free models providers offer that a pool does not have, and ones they no longer offer:
    /// `check` reports, `sync` rebuilds the generated targets file `serve` reads. Never edits your config.
    Models {
        #[command(subcommand)]
        command: ModelsCommand,
    },
    /// Print a shell completion script for bash, elvish, fish, powershell or zsh.
    Completions {
        /// The shell to generate completions for.
        shell: clap_complete::Shell,
    },
}

#[derive(clap::Args)]
struct CtlArgs {
    /// The gateway, like `https://gateway.example` or `http://127.0.0.1:8080`. Default: `HUMPYARD_URL`.
    #[arg(long, global = true)]
    url: Option<String>,
    /// The environment variable that holds your key (never a flag value, which would reach shell history).
    #[arg(long, global = true, default_value = "HUMPYARD_API_KEY")]
    key_env: String,
    /// Print the gateway's answer as JSON, unchanged.
    #[arg(long, global = true)]
    json: bool,
    /// Allow plain `http://` to another machine (the key would cross the network unprotected).
    #[arg(long, global = true)]
    allow_insecure_http: bool,
    #[command(subcommand)]
    command: CtlCommand,
}

#[derive(Subcommand)]
enum CtlCommand {
    /// A one-screen summary: reachable, config version, endpoints being skipped, your key's spend.
    Status,
    /// The routes and targets your key may use.
    Models,
    /// Which provider endpoints are being skipped, and which config version is loaded.
    Health,
    /// How routes, targets or endpoints have performed.
    Stats {
        /// How far back, like 30m, 24h or 7d.
        #[arg(long)]
        window: Option<String>,
        /// route, target or endpoint.
        #[arg(long)]
        group_by: Option<String>,
        #[arg(long)]
        route: Option<String>,
        #[arg(long)]
        target: Option<String>,
    },
    /// What the router did for your recent requests.
    Decisions {
        #[arg(long)]
        limit: Option<u32>,
        #[arg(long)]
        route: Option<String>,
        #[arg(long)]
        rule: Option<String>,
        #[arg(long)]
        outcome: Option<String>,
        #[arg(long)]
        request_id: Option<String>,
    },
    /// Provider quota state.
    Quota,
    /// What a request would do, without sending it.
    Explain {
        /// The model (route) the client would ask for.
        #[arg(long)]
        model: Option<String>,
        /// Estimated prompt size.
        #[arg(long)]
        prompt_tokens: Option<u64>,
        #[arg(long)]
        tools: bool,
        #[arg(long)]
        images: bool,
        #[arg(long)]
        subagent: bool,
        /// A full request description as JSON, instead of the flags above.
        #[arg(long)]
        body: Option<String>,
    },
    /// The configuration the gateway is serving, with where each part came from. Needs a key with
    /// `read_config = true`.
    Config {
        /// Print only the fingerprint (to compare with `check-config` on the repository's files).
        #[arg(long)]
        fingerprint: bool,
    },
    /// Your key's limits, spend and budget state.
    Whoami,
    /// Runtime overrides: temporarily stop using a target or a provider, or cut a route to one target. Changing
    /// them needs a key with `manage_overrides = true`; listing needs that or `read_config`.
    Overrides {
        #[command(subcommand)]
        command: OverridesCommand,
    },
}

#[derive(Subcommand)]
enum OverridesCommand {
    /// The overrides in force.
    List {
        /// Also show the ones that have ended, from the last 30 days.
        #[arg(long)]
        history: bool,
    },
    /// Add an override. It always expires.
    Add(OverrideAddArgs),
    /// Remove an override that is still in force.
    Remove {
        /// The id `add` or `list` printed.
        id: i64,
    },
}

#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("what").required(true).multiple(false)
    .args(["disable_target", "drain_provider", "pin"])))]
struct OverrideAddArgs {
    /// Stop using this target for every route.
    #[arg(long)]
    disable_target: Option<String>,
    /// Stop using every endpoint of this provider.
    #[arg(long)]
    drain_provider: Option<String>,
    /// Cut a route to one of its targets, as ROUTE=TARGET.
    #[arg(long, value_name = "ROUTE=TARGET")]
    pin: Option<String>,
    /// Why, for the audit trail.
    #[arg(long)]
    reason: String,
    /// How long it lasts, like 90s, 30m, 2h or 1d. The gateway's default (4h unless configured) if omitted.
    #[arg(long = "for", value_name = "DURATION")]
    duration: Option<String>,
}

#[derive(clap::Args)]
struct InitArgs {
    /// A built-in provider to set up (groq, openrouter, nvidia); repeatable. Omit to be asked.
    #[arg(short, long)]
    provider: Vec<String>,
    /// Another OpenAI-compatible endpoint, as `NAME,BASE_URL,KEY_ENV,MODEL`; repeatable.
    #[arg(long)]
    custom: Vec<String>,
    /// Where to write the config. Never overwritten. Pass '-' to write to stdout.
    #[arg(short, long, default_value = "config.toml")]
    out: PathBuf,
    /// The address to listen on. Anything but loopback also creates a key with a daily token budget.
    #[arg(long, default_value = humpyard::init::default_listen())]
    listen: String,
    /// Do not send the test request.
    #[arg(long)]
    no_test_request: bool,
}

#[derive(Subcommand)]
enum ModelsCommand {
    /// Report new, gone and drifted models for every provider that sets `discover`; exits 0 when there is
    /// nothing to review, 1 when there is, 2 when a source or the config could not be used.
    Check(ModelsArgs),
    /// Rebuild the generated targets file (`[discovery] file`) from the sources, only when it changes.
    /// Exits 0 on success, 1 when a guardrail refused the run, 2 when a source or the config could not be used.
    Sync(SyncArgs),
}

#[derive(clap::Args)]
struct ModelsArgs {
    /// The configuration file.
    #[arg(short, long)]
    config: PathBuf,
    /// Only this provider (its name in `[providers.<name>]`).
    #[arg(short, long)]
    provider: Option<String>,
    /// Print the report as JSON.
    #[arg(long)]
    json: bool,
    /// Also report new paid chat models (with their price), for providers on a paid plan; they are never
    /// added automatically, and finding one makes the exit code 1.
    #[arg(long)]
    paid: bool,
    /// Where the dataset is read (testing); https only, or http to a loopback host.
    #[arg(long, hide = true, default_value = prices::MODELS_DEV_URL)]
    source_url: String,
}

#[derive(clap::Args)]
struct SyncArgs {
    /// The configuration file.
    #[arg(short, long)]
    config: PathBuf,
    /// Only this provider.
    #[arg(short, long)]
    provider: Option<String>,
    /// Repeat on this interval (for example 6h) instead of running once; a failed run is logged and the
    /// loop goes on. For a sidecar or a second container where there is no scheduler.
    #[arg(long)]
    every: Option<String>,
    /// Where the dataset is read (testing); https only, or http to a loopback host.
    #[arg(long, hide = true, default_value = prices::MODELS_DEV_URL)]
    source_url: String,
}

#[derive(clap::Args)]
#[allow(clippy::doc_markdown)] // "ModelRelay" is a product name (these are also the --help text)
struct MigrateArgs {
    /// ModelRelay's base URL (https, or http on this machine), read through its admin API.
    #[arg(long, requires = "token_env", conflicts_with_all = ["config", "models"])]
    url: Option<String>,
    /// The environment variable that holds ModelRelay's admin token (never pass the token itself).
    #[arg(long)]
    token_env: Option<String>,
    /// A saved `/api/config` response, instead of --url.
    #[arg(long, requires = "models")]
    config: Option<PathBuf>,
    /// A saved `/api/models` response, instead of --url.
    #[arg(long, requires = "config")]
    models: Option<PathBuf>,
    /// The `class` given to every generated target.
    #[arg(long, default_value = "public")]
    class: String,
    /// Also generate `+min_ctx:N` routes for this minimum context window (repeatable).
    #[arg(long = "min-ctx")]
    min_ctx: Vec<u64>,
    /// Write the fragment here instead of stdout; refused if the file exists.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Declare ModelRelay's estimated quality scores too (by default only measured ones become `quality`).
    #[arg(long)]
    estimated_quality: bool,
    /// Name a ModelRelay provider differently in the fragment, as `FROM=TO` (repeatable), for example
    /// `--rename opencode=opencode-zen` when your config already has a provider called `opencode`.
    #[arg(long, value_parser = parse_rename)]
    rename: Vec<(String, String)>,
    /// Write `quality_weight = W` (0 to 1) on every generated `fastest` route, so pools prefer models with a
    /// declared quality.
    #[arg(long)]
    quality_weight: Option<f64>,
}

fn parse_rename(text: &str) -> Result<(String, String), String> {
    text.split_once('=')
        .filter(|(from, to)| !from.is_empty() && !to.is_empty())
        .map(|(from, to)| (from.to_string(), to.to_string()))
        .ok_or_else(|| format!("`{text}` is not FROM=TO"))
}

#[derive(clap::Args)]
struct PriceArgs {
    /// The config file whose endpoints are priced.
    #[arg(short, long)]
    config: PathBuf,
    /// Which provider of the config to check (its name in `[providers.<name>]`). Without it, every
    /// provider that sets `price_source` is checked.
    #[arg(short, long)]
    provider: Option<String>,
    /// Where prices come from, `models-dev:<provider id>`, overriding the provider's
    /// `price_source`. Needs `--provider`.
    #[arg(short, long)]
    source: Option<String>,
    /// Override the source URL (testing); https only, or http to a loopback host.
    #[arg(long, hide = true, default_value = prices::MODELS_DEV_URL)]
    url: String,
}

#[derive(Subcommand)]
enum PricesCommand {
    /// Print the `price` entries the source proposes for the provider's endpoints.
    Import(PriceArgs),
    /// Report endpoints whose price differs from, is missing in, or is gone from the source.
    /// Exit code 0: no drift, 1: drift, 2: the source or config could not be read.
    Check {
        #[command(flatten)]
        args: PriceArgs,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Exit code for a source or config that could not be used (distinct from drift).
const EXIT_UNUSABLE: u8 = 2;

/// The (provider, source) pairs to compare: what the flags name, or every provider of the config
/// that sets a `price_source`.
fn price_plan(
    args: &PriceArgs,
    config: &Config,
) -> Result<Vec<(String, String)>, prices::PriceError> {
    let fail = |message: String| Err(prices::PriceError::Config(message));
    match (&args.provider, &args.source) {
        (None, Some(_)) => fail("--source needs --provider".into()),
        (Some(provider), source) => {
            let Some(configured) = config.providers.get(provider) else {
                return fail(format!("the config has no provider `{provider}`"));
            };
            match source.as_ref().or(configured.price_source.as_ref()) {
                Some(source) => Ok(vec![(provider.clone(), source.clone())]),
                None => fail(format!(
                    "provider `{provider}` has no `price_source`; set one in the config or pass --source"
                )),
            }
        }
        (None, None) => {
            let plan: Vec<_> = config
                .providers
                .iter()
                .filter_map(|(name, p)| Some((name.clone(), p.price_source.clone()?)))
                .collect();
            if plan.is_empty() {
                return fail(
                    "no provider sets `price_source`; set one in the config, or pass --provider and --source"
                        .into(),
                );
            }
            Ok(plan)
        }
    }
}

async fn load_prices(args: &PriceArgs) -> Result<Vec<prices::Report>, prices::PriceError> {
    // Keys are not needed to compare prices: any value satisfies the env lookup.
    let text = std::fs::read_to_string(&args.config)
        .map_err(|e| prices::PriceError::Config(format!("{}: {e}", args.config.display())))?;
    let config = Config::from_toml(&text, |_| Some("unused".into()))
        .map_err(|e| prices::PriceError::Config(e.to_string()))?;
    let plan = price_plan(args, &config)?;
    // Validate every source before fetching anything, then fetch the document once.
    let ids = plan
        .iter()
        .map(|(_, spec)| prices::parse_source(spec))
        .collect::<Result<Vec<_>, _>>()?;
    let body = prices::fetch(&args.url).await?;
    plan.iter()
        .zip(ids)
        .map(|((provider, _), id)| {
            let data = prices::parse_models_dev(&body, id)?;
            Ok(prices::Report {
                provider: provider.clone(),
                rows: prices::compare(&config, provider, &data),
                source: data.source,
            })
        })
        .collect()
}

/// `models check` and `models sync` (ADR 0022).
async fn models_command(command: ModelsCommand) -> ExitCode {
    match command {
        ModelsCommand::Check(args) => {
            match discover::gather(
                &args.config,
                &args.source_url,
                args.provider.as_deref(),
                args.paid,
            )
            .await
            {
                Ok(gathered) => {
                    if args.json {
                        println!("{}", discover::report_json(&gathered.plan));
                    } else {
                        print!("{}", discover::render_report(&gathered.plan));
                    }
                    ExitCode::from(u8::from(gathered.plan.needs_review()))
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    ExitCode::from(EXIT_UNUSABLE)
                }
            }
        }
        ModelsCommand::Sync(args) => sync_command(args).await,
    }
}

/// One `models sync` run, or a loop of them with `--every`.
async fn sync_command(args: SyncArgs) -> ExitCode {
    let Some(every) = args.every.as_deref() else {
        return sync_once(&args).await;
    };
    let every = match discover::parse_every(every) {
        Ok(every) => every,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(EXIT_UNUSABLE);
        }
    };
    loop {
        // A failed run is logged and the loop goes on: the next one may find the sources back.
        let _ = sync_once(&args).await;
        tokio::time::sleep(every).await;
    }
}

async fn sync_once(args: &SyncArgs) -> ExitCode {
    let gathered = match discover::gather(
        &args.config,
        &args.source_url,
        args.provider.as_deref(),
        false,
    )
    .await
    {
        Ok(gathered) => gathered,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(EXIT_UNUSABLE);
        }
    };
    print!("{}", discover::render_report(&gathered.plan));
    match discover::apply(&gathered) {
        Ok(discover::SyncOutcome::Written) => {
            println!(
                "wrote {} generated targets ({} added, {} removed)",
                gathered.plan.desired.len(),
                gathered.plan.added.len(),
                gathered.plan.removed.len()
            );
            ExitCode::SUCCESS
        }
        Ok(discover::SyncOutcome::Unchanged) => {
            println!(
                "unchanged: {} generated targets",
                gathered.plan.desired.len()
            );
            ExitCode::SUCCESS
        }
        Ok(discover::SyncOutcome::Refused(reason)) => {
            eprintln!("refused: {reason}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(EXIT_UNUSABLE)
        }
    }
}

/// `migrate-modelrelay`: read the two `ModelRelay` responses, print the fragment, report on stderr.
async fn migrate_command(args: MigrateArgs) -> Result<(), String> {
    let (config_text, models_text) = match (&args.url, &args.config, &args.models) {
        (Some(url), _, _) => {
            let token = match &args.token_env {
                Some(name) => Some(
                    std::env::var(name)
                        .ok()
                        .filter(|t| !t.is_empty())
                        .ok_or_else(|| format!("`{name}` is not set"))?,
                ),
                None => None,
            };
            (
                migrate::fetch(url, "/api/config", token.as_deref())
                    .await
                    .map_err(|e| e.to_string())?,
                migrate::fetch(url, "/api/models", token.as_deref())
                    .await
                    .map_err(|e| e.to_string())?,
            )
        }
        (None, Some(config), Some(models)) => (
            std::fs::read_to_string(config)
                .map_err(|e| format!("cannot read {}: {e}", config.display()))?,
            std::fs::read_to_string(models)
                .map_err(|e| format!("cannot read {}: {e}", models.display()))?,
        ),
        _ => return Err("give --url (with --token-env) or --config and --models".into()),
    };
    let providers = migrate::parse_config(&config_text).map_err(|e| e.to_string())?;
    let models = migrate::parse_models(&models_text).map_err(|e| e.to_string())?;
    let options = migrate::Options {
        class: args.class,
        min_ctx: args.min_ctx,
        estimated_quality: args.estimated_quality,
        rename: args.rename.into_iter().collect(),
        quality_weight: args.quality_weight,
    };
    migrate::check_options(&providers, &options)?;
    let migration = migrate::migrate(&providers, &models, &options);
    migrate::self_check(&migration.fragment)
        .map_err(|e| format!("the generated fragment does not load, so it was not written: {e}"))?;
    match &args.out {
        Some(path) => {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|e| {
                    format!(
                        "cannot create {}: {e} (an existing file is never overwritten)",
                        path.display()
                    )
                })?;
            file.write_all(migration.fragment.as_bytes())
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        }
        None => print!("{}", migration.fragment),
    }
    let r = &migration.report;
    eprintln!(
        "\nmigrated {} providers, {} targets, {} routes; the fragment loads.",
        r.providers, r.targets, r.routes
    );
    eprintln!(
        "{} of {} targets declare a quality (measured scores only; --estimated-quality includes ModelRelay's estimates).",
        r.with_quality, r.targets
    );
    if !r.skipped.is_empty() {
        eprintln!("not carried over:");
        for line in &r.skipped {
            eprintln!("  - {line}");
        }
    }
    if !r.todo.is_empty() {
        eprintln!("to do by hand:");
        for line in &r.todo {
            eprintln!("  - {line}");
        }
    }
    eprintln!(
        "set each api_key_env variable in the gateway's environment, then merge the fragment into your config."
    );
    Ok(())
}

async fn prices_command(command: PricesCommand) -> ExitCode {
    let (args, json, check) = match command {
        PricesCommand::Import(args) => (args, false, false),
        PricesCommand::Check { args, json } => (args, json, true),
    };
    let read_on = time::OffsetDateTime::now_utc().date().to_string();
    let reports = match load_prices(&args).await {
        Ok(reports) => reports,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(EXIT_UNUSABLE);
        }
    };
    if !check {
        for report in &reports {
            print!(
                "{}",
                prices::render_snippet(&report.rows, &report.source, &read_on)
            );
        }
        return ExitCode::SUCCESS;
    }
    if json {
        println!("{}", prices::render_json(&reports, &read_on));
    } else {
        for report in &reports {
            print!(
                "{}",
                prices::render_report(&report.rows, &report.source, &read_on)
            );
        }
    }
    if reports.iter().any(|r| prices::has_drift(&r.rows)) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Turns SIGINT (Ctrl-C) and, on Unix, SIGTERM into messages: the first starts a graceful
/// shutdown, a second ends the wait for in-flight requests.
fn termination_signals() -> tokio::sync::mpsc::Receiver<()> {
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    tokio::spawn(async move {
        #[cfg(unix)]
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        loop {
            #[cfg(unix)]
            {
                let sigterm = async {
                    match terminate.as_mut() {
                        Some(signal) => {
                            signal.recv().await;
                        }
                        None => std::future::pending::<()>().await,
                    }
                };
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    () = sigterm => {}
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
            if tx.send(()).await.is_err() {
                break;
            }
        }
    });
    rx
}

/// Turns SIGHUP into reload requests: each one makes the running gateway re-read its config file.
/// `None` where the platform has no SIGHUP.
fn reload_signals() -> Option<tokio::sync::mpsc::Receiver<()>> {
    #[cfg(unix)]
    {
        let mut hangup =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()).ok()?;
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            while hangup.recv().await.is_some() {
                // A full queue already has reloads pending, which cover this one.
                let _ = tx.try_send(());
            }
        });
        Some(rx)
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // `init` prints its own report; the in-process test request should not log into it.
                if matches!(std::env::args().nth(1).as_deref(), Some("init")) {
                    "warn".into()
                } else {
                    "info".into()
                }
            }),
        )
        .init();
    let command = Cli::parse().command;
    if let Command::Completions { shell } = command {
        return write_completions(shell, &mut std::io::stdout().lock());
    }
    if let Command::Prices { command } = command {
        return prices_command(command).await;
    }
    if let Command::Models { command } = command {
        return models_command(command).await;
    }
    if let Command::Init(args) = command {
        return init_command(args).await;
    }
    if let Command::Ctl(args) = command {
        return ctl_command(args).await;
    }
    if let Command::MigrateModelrelay(args) = command {
        return match migrate_command(args).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("error: {message}");
                ExitCode::FAILURE
            }
        };
    }
    match run(command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn ctl_command(args: CtlArgs) -> ExitCode {
    use humpyard::ctl::{Command as C, OverridesCommand as O, Settings};
    let settings = Settings {
        url: args.url,
        key_env: args.key_env,
        allow_insecure_http: args.allow_insecure_http,
        json: args.json,
    };
    let command = match args.command {
        CtlCommand::Status => C::Status,
        CtlCommand::Models => C::Models,
        CtlCommand::Health => C::Health,
        CtlCommand::Quota => C::Quota,
        CtlCommand::Whoami => C::Whoami,
        CtlCommand::Config { fingerprint } => C::Config { fingerprint },
        CtlCommand::Stats {
            window,
            group_by,
            route,
            target,
        } => C::Stats {
            window,
            group_by,
            route,
            target,
        },
        CtlCommand::Decisions {
            limit,
            route,
            rule,
            outcome,
            request_id,
        } => C::Decisions {
            limit,
            route,
            rule,
            outcome,
            request_id,
        },
        CtlCommand::Explain {
            model,
            prompt_tokens,
            tools,
            images,
            subagent,
            body,
        } => C::Explain {
            model,
            prompt_tokens,
            tools,
            images,
            subagent,
            body_file: body,
        },
        CtlCommand::Overrides { command } => C::Overrides(match command {
            OverridesCommand::List { history } => O::List { history },
            OverridesCommand::Add(a) => O::Add {
                disable_target: a.disable_target,
                drain_provider: a.drain_provider,
                pin: a.pin,
                reason: a.reason,
                duration: a.duration,
            },
            OverridesCommand::Remove { id } => O::Remove { id },
        }),
    };
    let env = |name: &str| std::env::var(name).ok();
    let result = match humpyard::ctl::connect(&settings, &env) {
        Ok(gateway) => {
            humpyard::ctl::run(&gateway, command, settings.json, &mut std::io::stdout()).await
        }
        Err(failure) => Err(failure),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprintln!("error: {}", failure.message());
            ExitCode::from(failure.exit_code())
        }
    }
}

async fn init_command(args: InitArgs) -> ExitCode {
    use std::io::IsTerminal;
    let is_stdout = args.out == std::path::Path::new("-");
    let options = humpyard::init::InitOptions {
        providers: args.provider,
        custom: args.custom,
        out: args.out,
        listen: args.listen,
        no_proof: args.no_test_request,
    };
    let env = |name: &str| std::env::var(name).ok();
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    let input: Option<&mut dyn std::io::BufRead> = if std::io::stdin().is_terminal() {
        Some(&mut lock)
    } else {
        None
    };
    let result = if is_stdout {
        humpyard::init::run(
            options,
            &env,
            input,
            &mut std::io::stderr(),
            &mut std::io::stdout(),
        )
        .await
    } else {
        humpyard::init::run(
            options,
            &env,
            input,
            &mut std::io::stdout(),
            &mut std::io::sink(),
        )
        .await
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run(command: Command) -> Result<(), String> {
    match command {
        Command::CheckConfig { path } => {
            let config = Config::load(&path).map_err(|e| e.to_string())?;
            println!("config ok");
            if let Some(fingerprint) = humpyard::server::config_fingerprint(&path, &config) {
                println!("fingerprint: {fingerprint}");
            }
            match &config.discovery_state {
                humpyard::config::DiscoveryState::Loaded(n) => {
                    println!("generated targets: {n} loaded");
                }
                humpyard::config::DiscoveryState::Ignored(reason) => {
                    println!("generated targets: file ignored ({reason})");
                }
                humpyard::config::DiscoveryState::Missing => {
                    println!("generated targets: file not there yet (no sync has run)");
                }
                humpyard::config::DiscoveryState::Empty => {
                    println!("generated targets: file is empty");
                }
                humpyard::config::DiscoveryState::Disabled => {
                    println!(
                        "generated targets: discovery is switched off ([discovery] enabled = false)"
                    );
                }
                humpyard::config::DiscoveryState::Off => {}
            }
            Ok(())
        }
        Command::Keygen { id } => {
            let (key, hash) = humpyard::auth::generate_key()
                .map_err(|e| format!("cannot gather randomness: {e}"))?;
            println!("Key (shown once; store it securely): {key}\n");
            println!("Add to your config:\n\n[keys.{id}]\nsha256 = \"{hash}\"");
            Ok(())
        }
        Command::Prices { .. }
        | Command::MigrateModelrelay(_)
        | Command::Models { .. }
        | Command::Init(_)
        | Command::Ctl(_)
        | Command::Completions { .. } => Ok(()), // handled in `main`
        Command::Serve { config: path } => {
            let config = Config::load(&path).map_err(|e| e.to_string())?;
            let listener = tokio::net::TcpListener::bind(config.listen)
                .await
                .map_err(|e| format!("cannot bind {}: {e}", config.listen))?;
            tracing::info!(listen = %config.listen, "serving");
            let reload = reload_signals().map(|triggers| server::Reload { path, triggers });
            server::run(config, listener, termination_signals(), reload).await
        }
    }
}

fn write_completions(shell: clap_complete::Shell, out: &mut dyn Write) -> ExitCode {
    let mut cmd = Cli::command();
    let mut buf = Vec::new();
    clap_complete::generate(shell, &mut cmd, "humpyard", &mut buf);
    match out.write_all(&buf).and_then(|()| out.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("failed to write completions: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod completion_writer_tests {
    use super::*;

    struct BrokenPipeWriter;
    impl Write for BrokenPipeWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn broken_pipe_is_reported_as_success() {
        let code = write_completions(clap_complete::Shell::Fish, &mut BrokenPipeWriter);
        assert_eq!(code, ExitCode::SUCCESS);
    }

    #[test]
    fn other_io_errors_are_reported_as_failure() {
        struct FailingWriter;
        impl Write for FailingWriter {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let code = write_completions(clap_complete::Shell::Fish, &mut FailingWriter);
        assert_eq!(code, ExitCode::FAILURE);
    }
}
