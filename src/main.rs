use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use humpyard::config::Config;
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

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    match run(Cli::parse().command).await {
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
            Config::load(&path).map_err(|e| e.to_string())?;
            println!("config ok");
            Ok(())
        }
        Command::Keygen { id } => {
            let (key, hash) = humpyard::auth::generate_key()
                .map_err(|e| format!("cannot gather randomness: {e}"))?;
            println!("Key (shown once; store it securely): {key}\n");
            println!("Add to your config:\n\n[keys.{id}]\nsha256 = \"{hash}\"");
            Ok(())
        }
        Command::Serve { config } => {
            let config = Config::load(&config).map_err(|e| e.to_string())?;
            let listener = tokio::net::TcpListener::bind(config.listen)
                .await
                .map_err(|e| format!("cannot bind {}: {e}", config.listen))?;
            tracing::info!(listen = %config.listen, "serving");
            server::run(config, listener, termination_signals()).await
        }
    }
}
