use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

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
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let command = Cli::parse().command;
    if let Command::Prices { command } = command {
        return prices_command(command).await;
    }
    if let Command::Models { command } = command {
        return models_command(command).await;
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

async fn run(command: Command) -> Result<(), String> {
    match command {
        Command::CheckConfig { path } => {
            let config = Config::load(&path).map_err(|e| e.to_string())?;
            println!("config ok");
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
        Command::Prices { .. } | Command::MigrateModelrelay(_) | Command::Models { .. } => Ok(()), // handled in `main`
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
